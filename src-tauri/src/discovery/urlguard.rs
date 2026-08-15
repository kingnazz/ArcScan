//! The gate every SSDP `LOCATION` has to pass before ArcScan connects to it.
//!
//! # Why this exists
//!
//! `LOCATION` is a URL chosen by whatever answered a multicast query. Anything
//! on the local link can answer, and nothing about the response is
//! authenticated. Fetching that URL naively turns ArcScan into a server-side
//! request forgery gadget: a hostile responder could point it at a cloud
//! metadata endpoint, at a service on the loopback interface, at a machine on a
//! VPN subnet, or at a public host it wants traffic sent to.
//!
//! # The rules
//!
//! Checked in two stages so both are testable without a network:
//!
//! * [`parse_location`] — syntax. Scheme, credentials, fragment, host shape,
//!   port, path characters and overall length.
//! * [`authorize`] — destination. Every address the host resolves to must sit
//!   inside the local IPv4 network the scan actually ran against.
//!
//! What is refused, and why:
//!
//! | Refused | Reason |
//! |---|---|
//! | anything but `http` | see the note on TLS below |
//! | `user:pass@host` | credentials in a URL are never part of a device description |
//! | a fragment | it cannot affect what is fetched, so its presence means the URL was crafted |
//! | an IPv6 literal | ArcScan has no IPv6 scope to validate the address against |
//! | loopback, link-local, multicast, broadcast, unspecified | not a scanned device |
//! | any address outside the scanned local network | including *other* private ranges |
//! | a name resolving to anything outside that network | one bad answer refuses the whole name |
//! | ports 0, 22, 23, 25, 465, 587 | never a description endpoint; a protocol worth not poking |
//! | a control character anywhere in the URL | request smuggling |
//! | a URL past [`MAX_URL_CHARS`] | nothing legitimate is that long |
//!
//! # No TLS, deliberately
//!
//! `https` is refused. A local device's description endpoint is served with a
//! self-signed certificate for an address, which nothing can verify; accepting
//! it would mean either shipping a TLS stack that skips verification — a
//! meaningless connection with a reassuring padlock — or asking the operator to
//! trust a certificate they have no way to check. Not fetching is the honest
//! answer, and the SSDP headers still yield a name and a manufacturer.
//!
//! # No rebinding window
//!
//! [`authorize`] returns the exact [`SocketAddr`] that was approved, and the
//! fetch connects to *that*, never to the hostname again. There is no second
//! resolution for a hostile DNS server to answer differently, so a name that
//! validated as local cannot become public between the check and the connect.
//!
//! # No redirects, no proxy
//!
//! The fetcher treats any 3xx as a refusal rather than following it, so a
//! device cannot bounce ArcScan somewhere this module never saw. It also builds
//! its own connection rather than going through a system proxy, which would
//! send the request to a host that was never validated.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use url::{Host, Url};

/// Longest `LOCATION` accepted, in characters.
pub const MAX_URL_CHARS: usize = 512;

/// Longest path-and-query accepted, in characters.
pub const MAX_PATH_CHARS: usize = 256;

/// Ports a device description is never served on, and that are better not
/// spoken to at all.
const REFUSED_PORTS: [u16; 6] = [0, 22, 23, 25, 465, 587];

/// Why a `LOCATION` was refused. The text is shown to a person, so it says what
/// was wrong rather than quoting the URL back at them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    TooLong,
    ControlCharacter,
    Unparsable,
    Scheme(String),
    Credentials,
    Fragment,
    NoHost,
    Ipv6Literal,
    Port(u16),
    PathTooLong,
    /// The host did not resolve to anything.
    Unresolvable,
    /// An address outside the scanned local network, with the address itself.
    NotLocal(IpAddr),
}

impl Rejection {
    pub fn reason(&self) -> String {
        match self {
            Rejection::TooLong => "the address is longer than ArcScan will follow".into(),
            Rejection::ControlCharacter => "the address contains control characters".into(),
            Rejection::Unparsable => "the address is not a valid URL".into(),
            Rejection::Scheme(s) => format!("only plain http descriptions are read, not {s}"),
            Rejection::Credentials => "the address carries embedded credentials".into(),
            Rejection::Fragment => "the address carries a fragment".into(),
            Rejection::NoHost => "the address names no host".into(),
            Rejection::Ipv6Literal => {
                "the address is IPv6, which ArcScan does not scan and cannot place".into()
            }
            Rejection::Port(p) => format!("port {p} is not a device-description port"),
            Rejection::PathTooLong => "the address path is longer than ArcScan will follow".into(),
            Rejection::Unresolvable => "the host name did not resolve".into(),
            Rejection::NotLocal(ip) => {
                format!("{ip} is not on the local network this scan ran against")
            }
        }
    }
}

/// A `LOCATION` that passed the syntax stage. Still not safe to fetch: the
/// destination has not been placed on the network yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// An address literal, when the URL used one.
    pub literal: Option<Ipv4Addr>,
    /// The host as written, used for the `Host` header and for resolution.
    pub host: String,
    pub port: u16,
    /// Path plus query, already percent-encoded by the URL parser.
    pub path_and_query: String,
}

impl Parsed {
    /// True when the host is a literal address, so no name resolution is needed
    /// and no rebinding is even conceivable.
    pub fn is_literal(&self) -> bool {
        self.literal.is_some()
    }
}

/// A destination that has been both parsed and placed on the local network.
/// The only thing the fetcher is allowed to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedUrl {
    /// The exact address the fetch must use. Never resolved again.
    pub addr: SocketAddr,
    /// The value for the `Host` header, so a device serving several names still
    /// answers correctly.
    pub host_header: String,
    pub path_and_query: String,
}

/// The IPv4 ranges a description fetch may reach: the local subnets the scan
/// actually ran against, as (network, mask) pairs.
#[derive(Debug, Clone, Default)]
pub struct LocalPolicy {
    ranges: Vec<(u32, u32)>,
}

impl LocalPolicy {
    /// Build a policy from CIDR-style (network address, prefix length) pairs.
    pub fn from_networks(networks: &[(Ipv4Addr, u8)]) -> Self {
        let ranges = networks
            .iter()
            .filter(|(_, prefix)| *prefix > 0 && *prefix <= 32)
            .map(|(ip, prefix)| {
                let mask = if *prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix)
                };
                (u32::from(*ip) & mask, mask)
            })
            .collect();
        LocalPolicy { ranges }
    }

    /// True when `ip` is inside one of the scanned local networks *and* is an
    /// ordinary unicast address.
    ///
    /// The special-address checks come first and are not negotiable: a scanned
    /// range can legitimately contain the broadcast address, and `127.0.0.1`
    /// would be inside a hand-entered `127.0.0.0/8` target, but neither is a
    /// device description endpoint.
    pub fn allows(&self, ip: Ipv4Addr) -> bool {
        if ip.is_loopback()
            || ip.is_link_local()
            || ip.is_multicast()
            || ip.is_broadcast()
            || ip.is_unspecified()
        {
            return false;
        }
        let v = u32::from(ip);
        self.ranges.iter().any(|(net, mask)| v & mask == *net)
    }
}

/// Stage one: everything that can be decided from the text of the URL alone.
pub fn parse_location(raw: &str) -> Result<Parsed, Rejection> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > MAX_URL_CHARS {
        return Err(Rejection::TooLong);
    }
    // Checked before parsing: a CR or LF is how a header would be smuggled into
    // the request, and the parser would quietly strip some of them.
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(Rejection::ControlCharacter);
    }

    let url = Url::parse(trimmed).map_err(|_| Rejection::Unparsable)?;

    // Scheme comparison is already lowercased by the parser, so `HTTP://` and
    // `HtTp://` both arrive here as `http`.
    if url.scheme() != "http" {
        return Err(Rejection::Scheme(url.scheme().to_string()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Rejection::Credentials);
    }
    if url.fragment().is_some() {
        return Err(Rejection::Fragment);
    }

    let host = url.host().ok_or(Rejection::NoHost)?;
    let (literal, host_text) = match host {
        // The parser normalises every legal numeric form — `0x7f.0.0.1`,
        // `2130706433`, `0177.0.0.1` — into a real address before we see it, so
        // an unusual spelling cannot slip past the range check below.
        Host::Ipv4(ip) => (Some(ip), ip.to_string()),
        Host::Ipv6(_) => return Err(Rejection::Ipv6Literal),
        Host::Domain(name) => {
            if name.is_empty() {
                return Err(Rejection::NoHost);
            }
            (None, name.to_string())
        }
    };

    let port = url.port().unwrap_or(80);
    if REFUSED_PORTS.contains(&port) {
        return Err(Rejection::Port(port));
    }

    let mut path_and_query = url.path().to_string();
    if let Some(query) = url.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }
    if path_and_query.is_empty() {
        path_and_query.push('/');
    }
    if path_and_query.chars().count() > MAX_PATH_CHARS {
        return Err(Rejection::PathTooLong);
    }
    // The parser percent-encodes control characters in a path, so this is a
    // belt-and-braces assertion rather than the primary defence.
    if path_and_query.chars().any(|c| c.is_control() || c == ' ') {
        return Err(Rejection::ControlCharacter);
    }

    Ok(Parsed {
        literal,
        host: host_text,
        port,
        path_and_query,
    })
}

/// Stage two: place the destination on the network.
///
/// `resolved` is every address the host name resolved to, and is ignored for a
/// literal. **All** of them must be allowed — a name that answers with one local
/// address and one public address is refused outright, because which one a later
/// connection would use is not something this code gets to decide.
pub fn authorize(
    parsed: &Parsed,
    resolved: &[IpAddr],
    policy: &LocalPolicy,
) -> Result<ValidatedUrl, Rejection> {
    let addr = match parsed.literal {
        Some(ip) => {
            if !policy.allows(ip) {
                return Err(Rejection::NotLocal(IpAddr::V4(ip)));
            }
            ip
        }
        None => {
            if resolved.is_empty() {
                return Err(Rejection::Unresolvable);
            }
            let mut chosen: Option<Ipv4Addr> = None;
            for ip in resolved {
                match ip {
                    IpAddr::V4(v4) => {
                        if !policy.allows(*v4) {
                            return Err(Rejection::NotLocal(*ip));
                        }
                        if chosen.is_none() {
                            chosen = Some(*v4);
                        }
                    }
                    // A name that also answers with an IPv6 address is refused
                    // rather than silently preferring the IPv4 one: ArcScan
                    // cannot place the v6 address, and a resolver elsewhere in
                    // the stack might prefer it.
                    IpAddr::V6(_) => return Err(Rejection::NotLocal(*ip)),
                }
            }
            chosen.ok_or(Rejection::Unresolvable)?
        }
    };

    let host_header = if parsed.port == 80 {
        parsed.host.clone()
    } else {
        format!("{}:{}", parsed.host, parsed.port)
    };

    Ok(ValidatedUrl {
        addr: SocketAddr::new(IpAddr::V4(addr), parsed.port),
        host_header,
        path_and_query: parsed.path_and_query.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scan of 192.0.2.0/24 — the documentation range, used throughout so no
    /// test address can ever belong to a real network.
    fn policy() -> LocalPolicy {
        LocalPolicy::from_networks(&[("192.0.2.0".parse().unwrap(), 24)])
    }

    fn check(raw: &str) -> Result<ValidatedUrl, Rejection> {
        let parsed = parse_location(raw)?;
        authorize(&parsed, &[], &policy())
    }

    fn check_with(raw: &str, resolved: &[&str]) -> Result<ValidatedUrl, Rejection> {
        let parsed = parse_location(raw)?;
        let ips: Vec<IpAddr> = resolved.iter().map(|s| s.parse().unwrap()).collect();
        authorize(&parsed, &ips, &policy())
    }

    #[test]
    fn an_ordinary_local_description_url_is_allowed() {
        let ok = check("http://192.0.2.40:8080/rootDesc.xml").unwrap();
        assert_eq!(ok.addr, "192.0.2.40:8080".parse::<SocketAddr>().unwrap());
        assert_eq!(ok.host_header, "192.0.2.40:8080");
        assert_eq!(ok.path_and_query, "/rootDesc.xml");
    }

    #[test]
    fn port_eighty_is_left_out_of_the_host_header() {
        let ok = check("http://192.0.2.40/desc.xml").unwrap();
        assert_eq!(ok.addr.port(), 80);
        assert_eq!(ok.host_header, "192.0.2.40");
    }

    #[test]
    fn an_unusual_but_legal_port_is_allowed() {
        // UPnP stacks routinely serve descriptions from the ephemeral range.
        assert!(check("http://192.0.2.40:49152/desc.xml").is_ok());
        assert!(check("http://192.0.2.40:1900/d.xml").is_ok());
    }

    #[test]
    fn a_query_string_is_preserved_and_a_missing_path_becomes_root() {
        assert_eq!(
            check("http://192.0.2.40/d.xml?v=2").unwrap().path_and_query,
            "/d.xml?v=2"
        );
        assert_eq!(check("http://192.0.2.40").unwrap().path_and_query, "/");
    }

    #[test]
    fn a_public_address_is_refused() {
        assert_eq!(
            check("http://93.184.216.34/desc.xml"),
            Err(Rejection::NotLocal("93.184.216.34".parse().unwrap()))
        );
    }

    #[test]
    fn loopback_is_refused_in_every_spelling_the_parser_accepts() {
        for raw in [
            "http://127.0.0.1/d.xml",
            "http://127.1/d.xml",
            "http://0x7f.0.0.1/d.xml",
            "http://2130706433/d.xml",
            "http://0177.0.0.1/d.xml",
        ] {
            let outcome = check(raw);
            assert!(
                matches!(outcome, Err(Rejection::NotLocal(_))),
                "{raw} was not refused: {outcome:?}"
            );
        }
    }

    #[test]
    fn link_local_and_metadata_addresses_are_refused() {
        assert!(matches!(
            check("http://169.254.169.254/latest/meta-data/"),
            Err(Rejection::NotLocal(_))
        ));
        assert!(matches!(
            check("http://169.254.1.1/d.xml"),
            Err(Rejection::NotLocal(_))
        ));
    }

    #[test]
    fn the_unspecified_and_broadcast_addresses_are_refused() {
        assert!(matches!(
            check("http://0.0.0.0/d.xml"),
            Err(Rejection::NotLocal(_))
        ));
        assert!(matches!(
            check("http://255.255.255.255/d.xml"),
            Err(Rejection::NotLocal(_))
        ));
        assert!(matches!(
            check("http://239.255.255.250/d.xml"),
            Err(Rejection::NotLocal(_))
        ));
    }

    #[test]
    fn a_different_private_subnet_is_refused() {
        // The whole point of scoping to the scanned network rather than to
        // "private addresses": a VPN or a second interface is not this scan.
        for raw in [
            "http://10.0.0.5/d.xml",
            "http://172.16.4.4/d.xml",
            "http://192.168.1.1/d.xml",
            // Adjacent to the scanned range, but outside it.
            "http://192.0.3.40/d.xml",
        ] {
            assert!(
                matches!(check(raw), Err(Rejection::NotLocal(_))),
                "{raw} was not refused"
            );
        }
    }

    #[test]
    fn a_name_resolving_to_a_public_address_is_refused() {
        assert!(matches!(
            check_with("http://device.example/d.xml", &["93.184.216.34"]),
            Err(Rejection::NotLocal(_))
        ));
    }

    #[test]
    fn a_name_resolving_to_a_local_address_is_allowed_and_pinned() {
        let ok = check_with("http://printer.local/desc.xml", &["192.0.2.44"]).unwrap();
        // The approved address is what the fetch uses; the name is only a
        // header. There is no second resolution to rebind.
        assert_eq!(ok.addr, "192.0.2.44:80".parse::<SocketAddr>().unwrap());
        assert_eq!(ok.host_header, "printer.local");
    }

    #[test]
    fn a_name_answering_with_both_a_local_and_a_public_address_is_refused() {
        // The classic rebinding answer. Refusing the whole name is the only
        // safe reading: ArcScan does not get to pick which record wins later.
        assert!(matches!(
            check_with(
                "http://rebind.example/d.xml",
                &["192.0.2.44", "93.184.216.34"]
            ),
            Err(Rejection::NotLocal(_))
        ));
        assert!(matches!(
            check_with(
                "http://rebind.example/d.xml",
                &["93.184.216.34", "192.0.2.44"]
            ),
            Err(Rejection::NotLocal(_))
        ));
    }

    #[test]
    fn a_name_answering_with_an_ipv6_address_is_refused() {
        assert!(matches!(
            check_with("http://dual.example/d.xml", &["192.0.2.44", "2001:db8::1"]),
            Err(Rejection::NotLocal(_))
        ));
    }

    #[test]
    fn a_name_that_does_not_resolve_is_refused() {
        assert_eq!(
            check_with("http://nowhere.example/d.xml", &[]),
            Err(Rejection::Unresolvable)
        );
    }

    #[test]
    fn embedded_credentials_are_refused() {
        assert_eq!(
            check("http://admin:hunter2@192.0.2.40/d.xml"),
            Err(Rejection::Credentials)
        );
        assert_eq!(
            check("http://admin@192.0.2.40/d.xml"),
            Err(Rejection::Credentials)
        );
    }

    #[test]
    fn a_fragment_is_refused() {
        assert_eq!(
            check("http://192.0.2.40/d.xml#frag"),
            Err(Rejection::Fragment)
        );
    }

    #[test]
    fn only_plain_http_is_accepted() {
        assert_eq!(
            check("https://192.0.2.40/d.xml"),
            Err(Rejection::Scheme("https".into()))
        );
        for raw in [
            "file:///etc/passwd",
            "ftp://192.0.2.40/d.xml",
            "gopher://192.0.2.40/",
            "data:text/xml,<root/>",
            "javascript:alert(1)",
        ] {
            assert!(
                matches!(
                    check(raw),
                    Err(Rejection::Scheme(_)) | Err(Rejection::Unparsable)
                ),
                "{raw} was not refused"
            );
        }
    }

    #[test]
    fn a_mixed_case_scheme_is_normalised_not_smuggled() {
        // `HTTP://` is the same scheme, and must be accepted on its merits
        // rather than falling through a case-sensitive comparison.
        assert!(check("HTTP://192.0.2.40/d.xml").is_ok());
        assert!(matches!(
            check("HtTpS://192.0.2.40/d.xml"),
            Err(Rejection::Scheme(_))
        ));
    }

    #[test]
    fn an_ipv6_literal_is_refused() {
        assert_eq!(
            check("http://[2001:db8::1]/d.xml"),
            Err(Rejection::Ipv6Literal)
        );
        assert_eq!(
            check("http://[::1]:8080/d.xml"),
            Err(Rejection::Ipv6Literal)
        );
        // An IPv4-mapped v6 literal is still a v6 literal.
        assert_eq!(
            check("http://[::ffff:192.0.2.40]/d.xml"),
            Err(Rejection::Ipv6Literal)
        );
    }

    #[test]
    fn refused_ports_are_refused_before_anything_is_resolved() {
        for port in [0u16, 22, 23, 25, 465, 587] {
            let raw = format!("http://192.0.2.40:{port}/d.xml");
            assert!(
                matches!(check(&raw), Err(Rejection::Port(_))),
                "port {port} was not refused"
            );
        }
    }

    #[test]
    fn control_characters_anywhere_are_refused() {
        assert_eq!(
            parse_location("http://192.0.2.40/d.xml\r\nX-Evil: 1"),
            Err(Rejection::ControlCharacter)
        );
        assert_eq!(
            parse_location("http://192.0.2.40\r\n/d.xml"),
            Err(Rejection::ControlCharacter)
        );
        assert_eq!(
            parse_location("http://192.0.2.40/\u{0}d.xml"),
            Err(Rejection::ControlCharacter)
        );
    }

    #[test]
    fn an_oversized_url_or_path_is_refused() {
        let long = format!("http://192.0.2.40/{}", "a".repeat(MAX_URL_CHARS));
        assert_eq!(parse_location(&long), Err(Rejection::TooLong));

        // Inside the URL cap but past the path cap.
        let path = format!("http://192.0.2.40/{}", "b".repeat(MAX_PATH_CHARS + 5));
        assert_eq!(parse_location(&path), Err(Rejection::PathTooLong));
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed_at() {
        for raw in ["", "   ", "not a url", "://192.0.2.40", "http://"] {
            assert!(
                matches!(
                    parse_location(raw),
                    Err(Rejection::Unparsable) | Err(Rejection::NoHost)
                ),
                "{raw:?} was not refused"
            );
        }
    }

    #[test]
    fn an_empty_policy_allows_nothing() {
        let parsed = parse_location("http://192.0.2.40/d.xml").unwrap();
        assert!(matches!(
            authorize(&parsed, &[], &LocalPolicy::default()),
            Err(Rejection::NotLocal(_))
        ));
    }

    #[test]
    fn every_rejection_explains_itself_in_words() {
        for rejection in [
            Rejection::TooLong,
            Rejection::ControlCharacter,
            Rejection::Unparsable,
            Rejection::Scheme("https".into()),
            Rejection::Credentials,
            Rejection::Fragment,
            Rejection::NoHost,
            Rejection::Ipv6Literal,
            Rejection::Port(22),
            Rejection::PathTooLong,
            Rejection::Unresolvable,
            Rejection::NotLocal("10.0.0.1".parse().unwrap()),
        ] {
            // Each reason is embedded mid-sentence by the caller ("A
            // description address was refused: ..."), so it has to read as a
            // clause: no leading capital that is not a value, and no full stop.
            let reason = rejection.reason();
            assert!(!reason.is_empty(), "{rejection:?}");
            assert!(!reason.ends_with('.'), "{reason}");
            let first = reason.chars().next().unwrap();
            assert!(
                first.is_lowercase() || first.is_ascii_digit(),
                "{reason} should not start with a capital"
            );
        }
    }
}
