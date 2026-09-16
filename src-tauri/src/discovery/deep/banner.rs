//! Service banners, read and nothing more.
//!
//! A handful of protocols greet a connection with a line of text before the
//! client says anything: SSH, FTP, SMTP, POP3 and IMAP all do. That greeting is
//! written by whoever built the service, and it frequently names the
//! implementation, the platform, and sometimes the distribution.
//!
//! ArcScan connects, reads the greeting, and disconnects. It sends nothing, so
//! it cannot log in, cannot enumerate and cannot be mistaken for an attempt at
//! either. A banner is treated as Low or Medium evidence throughout: it is a
//! string a device chose to print, and a device that prints `SSH-2.0-OpenSSH`
//! may be a server, a switch, a NAS or a doorbell.

use crate::discovery::model::{
    sanitize_field, Confidence, DiscoverySource, Evidence, EvidenceKind,
};

use super::fingerprint::match_signature;

/// Ports that greet a connection without being asked.
///
/// Only these are read. A port not on this list is not connected to for a
/// banner, because a protocol that waits for the client to speak first would
/// simply time out and cost the scan its budget for nothing.
pub const GREETING_PORTS: &[u16] = &[21, 22, 23, 25, 110, 143, 587];

/// Most banner bytes read before the connection is dropped.
pub const MAX_BANNER_BYTES: usize = 1024;

/// Clean one raw greeting into a single displayable line.
///
/// Only the first line is kept: an FTP server that prints a six-line policy
/// notice has said everything identifying in the first one, and the rest is a
/// paragraph of somebody's legal text in an inventory column.
pub fn parse_banner(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(&raw[..raw.len().min(MAX_BANNER_BYTES)]);
    let first = text.lines().next()?;
    sanitize_field(first)
}

/// The OS family an SSH banner implies, where it implies one unambiguously.
///
/// Conservative by construction: `OpenSSH` on its own says nothing about the
/// platform, and only the distribution and platform suffixes that OpenSSH
/// packagers actually add are read.
pub fn ssh_os_family(banner: &str) -> Option<&'static str> {
    let lower = banner.to_lowercase();
    if !lower.starts_with("ssh-") {
        return None;
    }
    for (needle, family) in [
        ("ubuntu", "linux"),
        ("debian", "linux"),
        ("raspbian", "linux"),
        ("freebsd", "bsd"),
        ("openbsd", "bsd"),
        ("netbsd", "bsd"),
        ("sun_ssh", "solaris"),
        ("windows", "windows"),
    ] {
        if lower.contains(needle) {
            return Some(family);
        }
    }
    None
}

/// Turn a banner into discovery evidence.
pub fn evidence(banner: &str, port: u16) -> Vec<Evidence> {
    let Some(clean) = sanitize_field(banner) else {
        return Vec::new();
    };
    let mut out = vec![Evidence::new(
        DiscoverySource::Banner,
        EvidenceKind::Banner,
        port.to_string(),
        &clean,
        // A banner is a string a device chose to print. It is worth recording
        // and never worth being sure about.
        Confidence::Low,
    )];

    if let Some(family) = ssh_os_family(&clean) {
        out.push(Evidence::new(
            DiscoverySource::Banner,
            EvidenceKind::OsFamily,
            "",
            family,
            Confidence::Medium,
        ));
    }

    if let Some(signature) = match_signature(&clean) {
        if let Some(manufacturer) = signature.manufacturer {
            out.push(Evidence::new(
                DiscoverySource::Banner,
                EvidenceKind::Manufacturer,
                "",
                manufacturer,
                // One step below what the signature would carry from a protocol
                // that declared it: a banner is the weakest place to read one.
                Confidence::Low,
            ));
        }
        if let Some(family) = signature.os_family {
            out.push(Evidence::new(
                DiscoverySource::Banner,
                EvidenceKind::OsFamily,
                "",
                family,
                Confidence::Low,
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_greeting_reduces_to_its_first_line() {
        assert_eq!(
            parse_banner(b"SSH-2.0-OpenSSH_9.6p1 Ubuntu-3\r\nmore\r\n").as_deref(),
            Some("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3")
        );
        assert_eq!(
            parse_banner(b"220 mail.example.com ESMTP Postfix\r\n").as_deref(),
            Some("220 mail.example.com ESMTP Postfix")
        );
    }

    #[test]
    fn a_multi_line_policy_notice_does_not_become_an_inventory_column() {
        let raw =
            b"220 FTP ready\r\nUnauthorised access is prohibited.\r\nAll activity is logged.\r\n";
        assert_eq!(parse_banner(raw).as_deref(), Some("220 FTP ready"));
    }

    #[test]
    fn an_empty_or_binary_greeting_yields_nothing() {
        assert_eq!(parse_banner(b""), None);
        assert_eq!(parse_banner(b"\r\n"), None);
        assert_eq!(parse_banner(b"   \r\n"), None);
    }

    #[test]
    fn a_long_greeting_is_bounded() {
        let raw = vec![b'A'; 8192];
        let parsed = parse_banner(&raw).unwrap();
        assert!(parsed.chars().count() <= crate::discovery::model::MAX_FIELD_CHARS);
    }

    #[test]
    fn control_characters_never_survive_into_a_banner() {
        let parsed = parse_banner(b"SSH-2.0-\x07\x00OpenSSH\r\n").unwrap();
        assert!(!parsed.chars().any(char::is_control));
    }

    #[test]
    fn a_distribution_suffix_establishes_a_family_and_not_a_version() {
        assert_eq!(
            ssh_os_family("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3"),
            Some("linux")
        );
        assert_eq!(
            ssh_os_family("SSH-2.0-OpenSSH_9.3 FreeBSD-20230719"),
            Some("bsd")
        );
        let found = evidence("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3", 22);
        let family = found
            .iter()
            .find(|e| e.kind == EvidenceKind::OsFamily)
            .unwrap();
        assert_eq!(family.value, "linux");
        // A banner never names a release.
        assert!(!found.iter().any(|e| e.kind == EvidenceKind::OsVersion
            || e.kind == EvidenceKind::OsBuild
            || e.kind == EvidenceKind::OsProduct));
    }

    #[test]
    fn a_bare_openssh_banner_claims_no_platform() {
        assert_eq!(ssh_os_family("SSH-2.0-OpenSSH_9.6"), None);
        assert_eq!(ssh_os_family("220 not ssh at all"), None);
    }

    #[test]
    fn a_banner_is_recorded_at_low_confidence() {
        let found = evidence("220 ProFTPD Server ready", 21);
        assert_eq!(found[0].confidence, Confidence::Low);
        assert_eq!(found[0].source, DiscoverySource::Banner);
    }

    #[test]
    fn a_recognised_vendor_in_a_banner_is_weaker_than_the_same_string_over_http() {
        // MikroTik's SSH banner names RouterOS. Worth recording, and a banner
        // is the weakest place to read a vendor from.
        let found = evidence("SSH-2.0-ROSSSH MikroTik", 22);
        let manufacturer = found
            .iter()
            .find(|e| e.kind == EvidenceKind::Manufacturer)
            .unwrap();
        assert_eq!(manufacturer.value, "MikroTik");
        assert_eq!(manufacturer.confidence, Confidence::Low);
    }

    #[test]
    fn only_protocols_that_greet_are_listed() {
        // Port 80 waits for the client to speak, so reading it for a banner
        // would spend the scan's time budget on a guaranteed timeout.
        assert!(!GREETING_PORTS.contains(&80));
        assert!(!GREETING_PORTS.contains(&443));
        assert!(GREETING_PORTS.contains(&22));
    }
}
