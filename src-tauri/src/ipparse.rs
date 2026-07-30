//! IP target parsing.
//!
//! Accepts CIDR (`192.168.1.0/24`), dashed ranges (`10.0.0.1-10.0.0.50` or the
//! short form `10.0.0.1-50`), and single IPs. All parsing is done here so the
//! backend validates targets independently of the frontend.

use std::net::Ipv4Addr;

/// Upper bound on how many addresses a single scan may enumerate. A /16 is the
/// practical ceiling for a LAN sweep; anything larger is almost certainly a
/// mistake and would exhaust memory/time.
pub const MAX_HOSTS: usize = 65_536;

/// Parse a target string into the concrete list of addresses to probe.
///
/// Returns an error string suitable for surfacing to the user.
pub fn parse_target(input: &str) -> Result<Vec<Ipv4Addr>, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Target is empty.".into());
    }

    let hosts = if let Some((base, prefix)) = s.split_once('/') {
        parse_cidr(base.trim(), prefix.trim())?
    } else if let Some((a, b)) = s.split_once('-') {
        parse_range(a.trim(), b.trim())?
    } else {
        vec![parse_ipv4(s)?]
    };

    if hosts.is_empty() {
        return Err("Target expands to zero hosts.".into());
    }
    if hosts.len() > MAX_HOSTS {
        return Err(format!(
            "Target expands to {} hosts, which exceeds the {} host limit. Narrow the range.",
            hosts.len(),
            MAX_HOSTS
        ));
    }
    Ok(hosts)
}

/// Canonical form of a target, used to decide whether two scans cover the same
/// ground and may therefore be compared.
///
/// Normalizing matters because the same network is written many ways.
/// `192.168.1.0/24` and `192.168.1.37/24` are the same subnet; `10.0.0.1-50` and
/// `10.0.0.1-10.0.0.50` are the same range. Comparing raw target strings would
/// treat those as unrelated networks and silently skip the comparison.
///
/// A single address, a range and a CIDR block are deliberately *not*
/// interchangeable even when they enumerate the same addresses, because their
/// scan histories mean different things to the operator.
pub fn canonical_key(input: &str) -> Result<String, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Target is empty.".into());
    }
    if let Some((base, prefix)) = s.split_once('/') {
        let ip = parse_ipv4(base.trim())?;
        let bits: u32 = prefix
            .trim()
            .parse()
            .map_err(|_| format!("`/{prefix}` is not a valid CIDR prefix length."))?;
        if bits > 32 {
            return Err("CIDR prefix length must be between 0 and 32.".into());
        }
        let mask: u32 = if bits == 0 {
            0
        } else {
            u32::MAX << (32 - bits)
        };
        let network = Ipv4Addr::from(u32::from(ip) & mask);
        return Ok(format!("cidr:{network}/{bits}"));
    }
    if let Some((a, b)) = s.split_once('-') {
        let start = parse_ipv4(a.trim())?;
        let end = range_end(start, b.trim())?;
        let (start, end) = if u32::from(start) <= u32::from(end) {
            (start, end)
        } else {
            (end, start)
        };
        return Ok(format!("range:{start}-{end}"));
    }
    Ok(format!("host:{}", parse_ipv4(s)?))
}

fn parse_ipv4(s: &str) -> Result<Ipv4Addr, String> {
    s.parse::<Ipv4Addr>()
        .map_err(|_| format!("`{s}` is not a valid IPv4 address."))
}

/// Resolve the end of a dashed range, which may be a full address
/// (`10.0.0.50`) or a bare last octet (`50`).
fn range_end(start: Ipv4Addr, b: &str) -> Result<Ipv4Addr, String> {
    if b.contains('.') {
        return parse_ipv4(b);
    }
    let last: u8 = b
        .parse()
        .map_err(|_| format!("`{b}` is not a valid range end (expected an IP or 0-255)."))?;
    let o = start.octets();
    Ok(Ipv4Addr::new(o[0], o[1], o[2], last))
}

fn parse_cidr(base: &str, prefix: &str) -> Result<Vec<Ipv4Addr>, String> {
    let ip = parse_ipv4(base)?;
    let bits: u32 = prefix
        .parse()
        .map_err(|_| format!("`/{prefix}` is not a valid CIDR prefix length."))?;
    if bits > 32 {
        return Err("CIDR prefix length must be between 0 and 32.".into());
    }
    let ip_u = u32::from(ip);
    let mask: u32 = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    let network = ip_u & mask;
    let broadcast = network | !mask;

    // For /31 and /32 there is no host/broadcast split — include every address.
    let (first, last) = if bits >= 31 {
        (network, broadcast)
    } else {
        // Skip the network and broadcast addresses for normal subnets.
        (network + 1, broadcast.saturating_sub(1))
    };

    let count = (last as u64 - first as u64 + 1) as usize;
    if count > MAX_HOSTS {
        return Err(format!(
            "/{bits} expands to {count} hosts, which exceeds the {MAX_HOSTS} host limit."
        ));
    }
    Ok((first..=last).map(Ipv4Addr::from).collect())
}

fn parse_range(a: &str, b: &str) -> Result<Vec<Ipv4Addr>, String> {
    let start = parse_ipv4(a)?;
    // The end can be a full IP (10.0.0.50) or a short last-octet (50).
    let end = range_end(start, b)?;

    let s = u32::from(start);
    let e = u32::from(end);
    if e < s {
        return Err("Range end is lower than the range start.".into());
    }
    let count = (e as u64 - s as u64 + 1) as usize;
    if count > MAX_HOSTS {
        return Err(format!(
            "Range expands to {count} hosts, which exceeds the {MAX_HOSTS} host limit."
        ));
    }
    Ok((s..=e).map(Ipv4Addr::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_ip() {
        assert_eq!(parse_target("192.168.1.5").unwrap().len(), 1);
    }

    #[test]
    fn cidr_24_excludes_network_and_broadcast() {
        let h = parse_target("192.168.1.0/24").unwrap();
        assert_eq!(h.len(), 254);
        assert_eq!(h[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(h[253], Ipv4Addr::new(192, 168, 1, 254));
    }

    #[test]
    fn cidr_32_is_single_host() {
        assert_eq!(parse_target("10.0.0.9/32").unwrap().len(), 1);
    }

    #[test]
    fn dashed_full_range() {
        let h = parse_target("10.0.0.1-10.0.0.50").unwrap();
        assert_eq!(h.len(), 50);
    }

    #[test]
    fn dashed_short_range() {
        let h = parse_target("10.0.0.1-50").unwrap();
        assert_eq!(h.len(), 50);
    }

    #[test]
    fn rejects_reversed_range() {
        assert!(parse_target("10.0.0.50-10.0.0.1").is_err());
    }

    #[test]
    fn rejects_oversized() {
        assert!(parse_target("10.0.0.0/8").is_err());
    }

    #[test]
    fn accepts_public_target() {
        // ArcScan scans whatever range you enter — public addresses included.
        assert_eq!(parse_target("8.8.8.8").unwrap().len(), 1);
    }

    #[test]
    fn canonical_key_normalizes_equivalent_cidrs() {
        // Any address inside the block yields the same key, so a scan typed as
        // `192.168.1.37/24` compares against one typed `192.168.1.0/24`.
        let expected = "cidr:192.168.1.0/24";
        assert_eq!(canonical_key("192.168.1.0/24").unwrap(), expected);
        assert_eq!(canonical_key("192.168.1.37/24").unwrap(), expected);
        assert_eq!(canonical_key(" 192.168.1.254/24 ").unwrap(), expected);
    }

    #[test]
    fn canonical_key_normalizes_short_and_long_ranges() {
        let expected = "range:10.0.0.1-10.0.0.50";
        assert_eq!(canonical_key("10.0.0.1-50").unwrap(), expected);
        assert_eq!(canonical_key("10.0.0.1-10.0.0.50").unwrap(), expected);
        // A reversed range covers the same ground, so it gets the same key.
        assert_eq!(canonical_key("10.0.0.50-10.0.0.1").unwrap(), expected);
    }

    #[test]
    fn canonical_key_keeps_target_shapes_distinct() {
        // These enumerate the same single address but mean different things in a
        // scan history, so they must not be compared against each other.
        assert_ne!(
            canonical_key("10.0.0.9").unwrap(),
            canonical_key("10.0.0.9/32").unwrap()
        );
        assert_ne!(
            canonical_key("10.0.0.1-10.0.0.1").unwrap(),
            canonical_key("10.0.0.1").unwrap()
        );
        assert_eq!(canonical_key("10.0.0.9").unwrap(), "host:10.0.0.9");
    }

    #[test]
    fn canonical_key_separates_different_networks() {
        assert_ne!(
            canonical_key("192.168.1.0/24").unwrap(),
            canonical_key("192.168.2.0/24").unwrap()
        );
        assert_ne!(
            canonical_key("192.168.1.0/24").unwrap(),
            canonical_key("192.168.1.0/25").unwrap()
        );
    }

    #[test]
    fn canonical_key_rejects_malformed_targets() {
        assert!(canonical_key("").is_err());
        assert!(canonical_key("not-an-ip").is_err());
        assert!(canonical_key("10.0.0.0/33").is_err());
        assert!(canonical_key("10.0.0.1-nope").is_err());
    }
}
