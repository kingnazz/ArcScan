//! TCP port specifications: parsing, validation, de-duplication, and the
//! service-name map.
//!
//! The backend is the source of truth for ports. The UI may pre-parse a spec to
//! give immediate feedback, but every scan runs through [`sanitize`] here, so a
//! malformed or oversized port list cannot reach the probe loop regardless of
//! what the frontend sent.

use std::collections::BTreeSet;

/// Upper bound on how many distinct ports a single scan may probe. 2,048 ports
/// across a /24 is already half a million connection attempts, which is the
/// practical ceiling for a desktop app that must stay responsive and must not
/// overwhelm consumer routers.
pub const MAX_PORTS: usize = 2_048;

/// Default TCP ports probed for liveness and quick service detection. A curated
/// spread of the ports that matter most on a typical LAN: small enough to stay
/// fast, wide enough to fingerprint most hosts.
pub const DEFAULT_PORTS: [u16; 14] = [
    21, 22, 23, 53, 80, 110, 139, 143, 443, 445, 3389, 5900, 8080, 8443,
];

/// Validate, de-duplicate and sort a port list coming from the frontend.
///
/// An empty list means "use the defaults". Port 0 is rejected rather than
/// silently dropped, because a caller asking for it has a bug the user should
/// see instead of a quietly different scan.
pub fn sanitize(ports: &[u16]) -> Result<Vec<u16>, String> {
    if ports.is_empty() {
        return Ok(DEFAULT_PORTS.to_vec());
    }
    if ports.contains(&0) {
        return Err("Port 0 is not a valid TCP port. Use 1 to 65535.".into());
    }
    let unique: BTreeSet<u16> = ports.iter().copied().collect();
    if unique.len() > MAX_PORTS {
        return Err(format!(
            "{} distinct ports selected, which exceeds the {MAX_PORTS} port limit. \
             Narrow the port range.",
            unique.len()
        ));
    }
    Ok(unique.into_iter().collect())
}

/// Parse a human-written port specification into a validated port list.
///
/// Accepts single ports, comma-separated lists, space-separated lists, ranges,
/// and any mix of those: `22`, `22,80,443`, `22 80 443`, `1-1024`,
/// `80, 443, 8000-8100`.
pub fn parse_spec(input: &str) -> Result<Vec<u16>, String> {
    let text = input.trim();
    if text.is_empty() {
        return Ok(DEFAULT_PORTS.to_vec());
    }
    let mut set: BTreeSet<u16> = BTreeSet::new();
    for raw in text.split(|c: char| c == ',' || c.is_whitespace()) {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        match token.split_once('-') {
            Some((a, b)) => {
                let start = parse_port(a.trim())?;
                let end = parse_port(b.trim())?;
                let (start, end) = if start <= end {
                    (start, end)
                } else {
                    (end, start)
                };
                // Reject before expanding so a `1-65535` spec cannot allocate
                // 64k entries just to be refused afterwards.
                let width = end as usize - start as usize + 1;
                if set.len() + width > MAX_PORTS {
                    return Err(format!(
                        "`{token}` expands to {width} ports, taking the selection past the \
                         {MAX_PORTS} port limit. Narrow the range."
                    ));
                }
                set.extend(start..=end);
            }
            None => {
                set.insert(parse_port(token)?);
                if set.len() > MAX_PORTS {
                    return Err(format!(
                        "More than {MAX_PORTS} ports selected. Narrow the port list."
                    ));
                }
            }
        }
    }
    if set.is_empty() {
        return Err("No valid ports in the port list.".into());
    }
    Ok(set.into_iter().collect())
}

fn parse_port(token: &str) -> Result<u16, String> {
    let n: u32 = token
        .parse()
        .map_err(|_| format!("`{token}` is not a port number."))?;
    if n == 0 || n > 65_535 {
        return Err(format!("`{token}` is out of range. Ports are 1 to 65535."));
    }
    Ok(n as u16)
}

/// Well-known service name for a port, used for display and for deciding which
/// device actions make sense. Deliberately a curated list of what shows up on
/// real networks rather than the full IANA registry.
pub fn service_name(port: u16) -> Option<&'static str> {
    let name = match port {
        20 | 21 => "FTP",
        22 => "SSH",
        23 => "Telnet",
        25 | 587 => "SMTP",
        53 => "DNS",
        67 | 68 => "DHCP",
        69 => "TFTP",
        80 => "HTTP",
        88 => "Kerberos",
        110 => "POP3",
        111 => "RPC",
        119 => "NNTP",
        123 => "NTP",
        135 => "MS RPC",
        137..=139 => "NetBIOS",
        143 => "IMAP",
        161 | 162 => "SNMP",
        389 => "LDAP",
        427 => "SLP",
        443 => "HTTPS",
        445 => "SMB",
        465 => "SMTPS",
        500 => "IKE",
        515 => "LPD",
        548 => "AFP",
        554 => "RTSP",
        631 => "IPP",
        636 => "LDAPS",
        993 => "IMAPS",
        995 => "POP3S",
        1080 => "SOCKS",
        1433 => "MSSQL",
        1521 => "Oracle",
        1723 => "PPTP",
        1883 => "MQTT",
        1900 => "SSDP",
        2049 => "NFS",
        2375 | 2376 => "Docker",
        3000 => "HTTP-dev",
        3128 => "Proxy",
        3306 => "MySQL",
        3389 => "RDP",
        4443 => "HTTPS-alt",
        5000 => "UPnP",
        5060 | 5061 => "SIP",
        5222 => "XMPP",
        5353 => "mDNS",
        5432 => "PostgreSQL",
        5555 => "ADB",
        5900..=5905 => "VNC",
        6379 => "Redis",
        7000 => "AirPlay",
        8000 | 8008 | 8081 => "HTTP-alt",
        8009 => "Cast",
        8080 => "HTTP-alt",
        8081..=8090 => "HTTP-alt",
        8123 => "Home Assistant",
        8443 => "HTTPS-alt",
        8883 => "MQTTS",
        9000 => "HTTP-alt",
        9100 => "Printer",
        9200 => "Elasticsearch",
        10000 => "Webmin",
        27017 => "MongoDB",
        32400 => "Plex",
        _ => return None,
    };
    Some(name)
}

/// Services that deserve a visual warning: remote control and file sharing
/// surfaces that are common but risky to leave exposed.
pub fn is_sensitive(port: u16) -> bool {
    matches!(port, 21 | 23 | 139 | 445 | 3389 | 5900..=5905 | 5555)
}

/// Every port this build knows a service name for, with its sensitivity flag.
///
/// The frontend fetches this once at startup instead of keeping its own copy of
/// the table, so a service name or warning added here shows up in the UI without
/// a second list to keep in sync.
pub fn catalog() -> Vec<(u16, &'static str, bool)> {
    (1..=u16::MAX)
        .filter_map(|port| service_name(port).map(|name| (port, name, is_sensitive(port))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_spec_falls_back_to_defaults() {
        assert_eq!(parse_spec("   ").unwrap(), DEFAULT_PORTS.to_vec());
        assert_eq!(sanitize(&[]).unwrap(), DEFAULT_PORTS.to_vec());
    }

    #[test]
    fn parses_single_port() {
        assert_eq!(parse_spec("443").unwrap(), vec![443]);
    }

    #[test]
    fn parses_comma_and_space_lists() {
        assert_eq!(parse_spec("22,80,443").unwrap(), vec![22, 80, 443]);
        assert_eq!(parse_spec("22 80 443").unwrap(), vec![22, 80, 443]);
        assert_eq!(parse_spec("22, 80  443,").unwrap(), vec![22, 80, 443]);
    }

    #[test]
    fn parses_ranges_and_mixed_specs() {
        assert_eq!(parse_spec("80-82").unwrap(), vec![80, 81, 82]);
        assert_eq!(
            parse_spec("443, 80-82, 22").unwrap(),
            vec![22, 80, 81, 82, 443]
        );
    }

    #[test]
    fn reversed_range_is_accepted_and_ordered() {
        assert_eq!(parse_spec("82-80").unwrap(), vec![80, 81, 82]);
    }

    #[test]
    fn deduplicates_and_sorts() {
        assert_eq!(parse_spec("443,80,443,80-81").unwrap(), vec![80, 81, 443]);
        assert_eq!(sanitize(&[443, 80, 443, 22]).unwrap(), vec![22, 80, 443]);
    }

    #[test]
    fn rejects_out_of_range_ports() {
        assert!(parse_spec("0").is_err());
        assert!(parse_spec("65536").is_err());
        assert!(parse_spec("-5").is_err());
        assert!(sanitize(&[0, 80]).is_err());
    }

    #[test]
    fn rejects_non_numeric_tokens() {
        assert!(parse_spec("http").is_err());
        assert!(parse_spec("80,http").is_err());
    }

    #[test]
    fn enforces_the_port_limit() {
        // A full 1-65535 sweep is refused up front rather than truncated.
        let err = parse_spec("1-65535").unwrap_err();
        assert!(err.contains(&MAX_PORTS.to_string()), "{err}");
        assert_eq!(parse_spec("1-2048").unwrap().len(), MAX_PORTS);
        assert!(parse_spec("1-2048,3000").is_err());

        let too_many: Vec<u16> = (1..=(MAX_PORTS as u16 + 1)).collect();
        assert!(sanitize(&too_many).is_err());
    }

    #[test]
    fn known_services_resolve() {
        assert_eq!(service_name(443), Some("HTTPS"));
        assert_eq!(service_name(3389), Some("RDP"));
        assert_eq!(service_name(5901), Some("VNC"));
        assert_eq!(service_name(64999), None);
    }

    #[test]
    fn catalog_covers_the_service_map() {
        let catalog = catalog();
        assert!(catalog.len() > 60, "{} entries", catalog.len());
        let https = catalog.iter().find(|(p, _, _)| *p == 443).unwrap();
        assert_eq!(https.1, "HTTPS");
        assert!(!https.2);
        let rdp = catalog.iter().find(|(p, _, _)| *p == 3389).unwrap();
        assert!(rdp.2, "RDP must be flagged as sensitive");
        // Ports with no service name are absent rather than listed as numbers.
        assert!(catalog.iter().all(|(p, _, _)| service_name(*p).is_some()));
        assert!(!catalog.iter().any(|(p, _, _)| *p == 64999));
    }

    #[test]
    fn sensitive_services_flagged() {
        assert!(is_sensitive(3389));
        assert!(is_sensitive(445));
        assert!(!is_sensitive(443));
    }
}
