//! Scan comparison signatures.
//!
//! Two scans may only be compared when they looked for the same things in the
//! same place. The *place* is the canonical target key from [`crate::ipparse`];
//! this module produces the *coverage key*: a stable, deterministic string
//! identifying everything that decides which devices and services a scan can
//! discover — the normalized port set and the ARP-assist strategy.
//!
//! Execution tuning (timeout, the three concurrency limits) deliberately stays
//! out of the coverage key. Those settings change how fast a scan runs and how
//! hard it pushes the network, not what it is looking for; folding them in
//! would make ordinary scans of the same profile incomparable whenever a
//! slider moved. They are recorded separately, as `execution_settings`, purely
//! for transparency.
//!
//! # Stability
//!
//! Coverage keys are persisted and compared across releases, so the format is
//! versioned (`v1|...`) and built by deterministic serialization — never by
//! hashing. Sorting and de-duplicating the ports first means `22,80,443`,
//! `443,80,22` and `22,22,80,443` all produce the same key, while consecutive
//! runs are compressed (`1-1024`) so a Full TCP range stays short. Do not
//! change the format of existing versions; add a new version prefix instead.

use serde::{Deserialize, Serialize};

/// Performance tuning recorded with a scan for transparency. Not part of the
/// comparison signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSettings {
    pub timeout_ms: u64,
    pub host_concurrency: usize,
    pub tcp_concurrency: usize,
    pub ping_concurrency: usize,
}

/// The discovery-mode component of the coverage key.
fn arp_mode(arp_assist: Option<bool>) -> &'static str {
    match arp_assist {
        None => "arp:auto",
        Some(true) => "arp:local",
        Some(false) => "arp:routed",
    }
}

/// Build the coverage key for a scan's detection-affecting settings.
pub fn coverage_key(ports: &[u16], arp_assist: Option<bool>) -> String {
    format!(
        "v1|{}|ports:{}",
        arp_mode(arp_assist),
        compress_ports(ports)
    )
}

/// Normalize a port list into a canonical compressed form: sorted, de-duplicated,
/// consecutive runs written as ranges. Empty input reads as `none`.
fn compress_ports(ports: &[u16]) -> String {
    let mut sorted: Vec<u16> = ports.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.is_empty() {
        return "none".into();
    }

    let mut parts: Vec<String> = Vec::new();
    let mut run_start = sorted[0];
    let mut run_end = sorted[0];
    for &port in &sorted[1..] {
        if port == run_end + 1 {
            run_end = port;
        } else {
            parts.push(format_run(run_start, run_end));
            run_start = port;
            run_end = port;
        }
    }
    parts.push(format_run(run_start, run_end));
    parts.join(",")
}

fn format_run(start: u16, end: u16) -> String {
    if start == end {
        format!("{start}")
    } else {
        format!("{start}-{end}")
    }
}

/// The port set the Quick LAN and Remote subnet profiles used in v1.6/v1.7.0,
/// mirrored from the frontend profile table those releases shipped.
const LEGACY_DEFAULT_PORTS: [u16; 14] = crate::ports::DEFAULT_PORTS;

/// The Reliable LAN port set as shipped in v1.7.0.
const LEGACY_RELIABLE_PORTS: [u16; 22] = [
    21, 22, 23, 53, 80, 135, 139, 143, 443, 445, 515, 548, 554, 631, 1900, 3389, 5000, 5353, 5900,
    8080, 8443, 9100,
];

/// Derive the best available coverage key for a scan saved before coverage keys
/// existed, from its stored profile id.
///
/// The fixed profiles (Quick LAN, Reliable LAN, Remote subnet) always ran with
/// their published port set and discovery mode, so their coverage is known
/// exactly. `Custom` and `Full TCP` scans ran with whatever ports the operator
/// had configured at the time, which was never persisted — those are given a
/// key unique to the scan, so they compare with nothing rather than comparing
/// wrongly. The same applies to scans with no profile at all (v1.6).
pub fn legacy_coverage_key(profile: Option<&str>, scan_id: i64) -> String {
    match profile {
        Some("quick-lan") => coverage_key(&LEGACY_DEFAULT_PORTS, None),
        Some("reliable-lan") => coverage_key(&LEGACY_RELIABLE_PORTS, None),
        Some("remote-subnet") => coverage_key(&LEGACY_DEFAULT_PORTS, Some(false)),
        _ => format!("legacy:scan:{scan_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_key_is_deterministic() {
        let a = coverage_key(&[22, 80, 443], None);
        let b = coverage_key(&[22, 80, 443], None);
        assert_eq!(a, b);
        // The format is persisted; a change here breaks comparison of existing
        // history and must be a deliberate, versioned decision.
        assert_eq!(a, "v1|arp:auto|ports:22,80,443");
    }

    #[test]
    fn port_order_and_duplicates_do_not_change_the_key() {
        let canonical = coverage_key(&[22, 80, 443], None);
        assert_eq!(coverage_key(&[443, 22, 80], None), canonical);
        assert_eq!(coverage_key(&[22, 22, 80, 443, 443], None), canonical);
    }

    #[test]
    fn different_port_sets_produce_different_keys() {
        assert_ne!(
            coverage_key(&[22, 80, 443], None),
            coverage_key(&[22], None)
        );
        assert_ne!(
            coverage_key(&[1, 2, 3], None),
            coverage_key(&[1, 2, 4], None)
        );
    }

    #[test]
    fn arp_strategy_is_part_of_the_key() {
        let ports = [22, 80];
        let auto = coverage_key(&ports, None);
        let local = coverage_key(&ports, Some(true));
        let routed = coverage_key(&ports, Some(false));
        assert_ne!(auto, routed);
        assert_ne!(auto, local);
        assert_ne!(local, routed);
    }

    #[test]
    fn consecutive_ports_compress_into_ranges() {
        assert_eq!(
            coverage_key(&(1..=1024).collect::<Vec<u16>>(), None),
            "v1|arp:auto|ports:1-1024"
        );
        assert_eq!(
            coverage_key(&[1, 2, 3, 5, 9, 10], Some(false)),
            "v1|arp:routed|ports:1-3,5,9-10"
        );
    }

    #[test]
    fn empty_port_list_reads_as_none() {
        assert_eq!(coverage_key(&[], None), "v1|arp:auto|ports:none");
    }

    #[test]
    fn legacy_fixed_profiles_derive_their_published_coverage() {
        assert_eq!(
            legacy_coverage_key(Some("quick-lan"), 7),
            coverage_key(&LEGACY_DEFAULT_PORTS, None)
        );
        assert_eq!(
            legacy_coverage_key(Some("remote-subnet"), 7),
            coverage_key(&LEGACY_DEFAULT_PORTS, Some(false))
        );
        assert_eq!(
            legacy_coverage_key(Some("reliable-lan"), 7),
            coverage_key(&LEGACY_RELIABLE_PORTS, None)
        );
    }

    #[test]
    fn legacy_uncertain_scans_get_a_key_unique_to_the_scan() {
        // Custom and Full TCP port sets were never persisted, so these scans
        // must fail safely: comparable with nothing, including each other.
        assert_ne!(
            legacy_coverage_key(Some("custom"), 1),
            legacy_coverage_key(Some("custom"), 2)
        );
        assert_ne!(
            legacy_coverage_key(Some("full-tcp"), 3),
            legacy_coverage_key(Some("full-tcp"), 4)
        );
        assert_ne!(legacy_coverage_key(None, 5), legacy_coverage_key(None, 6));
    }

    #[test]
    fn quick_lan_and_reliable_lan_never_share_a_key() {
        assert_ne!(
            legacy_coverage_key(Some("quick-lan"), 1),
            legacy_coverage_key(Some("reliable-lan"), 1)
        );
    }
}
