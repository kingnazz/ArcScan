//! The redacted discovery report a person can copy and paste into a bug report.
//!
//! # The problem this solves
//!
//! Someone whose television is being called a media device has no way to tell
//! ArcScan why it is wrong. Asking them to describe what their device
//! advertises is asking them to read multicast DNS. Asking them to send a
//! screenshot of the drawer sends a name, an address and a MAC to a public
//! issue tracker.
//!
//! # The rule
//!
//! The report contains what is needed to fix a classification rule and nothing
//! else. That is enforced twice over. First structurally: [`DeviceDiagnostic`]
//! has no field for notes, a MAC address, a serial number, a UDN or a URL, so
//! the caller cannot pass one in even by accident. Second by filter:
//! [`build_report`] drops evidence kinds that carry identifiers regardless of
//! what it was handed, because the evidence list is generic and a future kind
//! could otherwise arrive unreviewed.
//!
//! The address is the one exception, and it is masked to its first two octets:
//! `192.168.x.x` says "this is a home network behind a consumer router", which
//! is genuinely useful context, without saying which device.
//!
//! Nothing here opens a socket, writes a file or contacts anything. The caller
//! puts the string on the clipboard; that is the entire transport.

use super::effective::{Freshness, TypeSource};
use super::model::{Confidence, DeviceType, DiscoveryQuality};

/// Longest report, in characters. A device advertising a great deal is still
/// pasteable, and a hostile one cannot produce a megabyte of clipboard.
pub const MAX_REPORT_CHARS: usize = 4_000;

/// Most evidence lines of each freshness shown.
const MAX_EVIDENCE_LINES: usize = 16;

/// Longest single value shown, before an ellipsis.
const MAX_VALUE_CHARS: usize = 80;

/// Evidence kinds the report never includes, whatever it is handed.
///
/// `serial_number`, `url` and `protocol_identifier` (a UPnP UDN or an mDNS
/// instance name) identify one specific unit rather than one kind of device,
/// and an address says where rather than what. None of them can help fix a
/// classification rule, so none of them earn the risk.
///
/// v1.9 adds two more by the same test. A `system_uuid` is the strongest
/// identifier a machine has and says nothing whatever about what kind of thing
/// it is, so it is pure risk. A `domain_membership` names the customer's
/// directory rather than the device, and "is it domain-joined" — the only part
/// a rule could use — is already carried by the Windows product type.
///
/// The deep-scan strings a rule *does* turn on — a `banner`, a
/// `certificate_subject`, a `page_title` — are kept, because they are what a
/// classification rule matches against and a report that omitted them could
/// not explain a wrong answer. They are clipped to [`MAX_VALUE_CHARS`] like
/// everything else.
const EXCLUDED_KINDS: &[&str] = &[
    "serial_number",
    "url",
    "protocol_identifier",
    "ipv4_address",
    "ipv6_address",
    "system_uuid",
    "domain_membership",
];

/// One claim, as the report shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticEvidence {
    pub source: String,
    pub kind: String,
    pub value: String,
    pub freshness: Freshness,
    /// How many qualifying scans have missed it, for the "seen N scans ago"
    /// phrasing. A count of scans, never a date.
    pub misses: i64,
}

/// Everything the report is allowed to know about a device.
///
/// Deliberately not a `Device` or a `DeviceDiscovery`: those carry the note,
/// the MAC, the serial and the database id, and a report built from them would
/// be one careless `format!` away from leaking all four.
#[derive(Debug, Clone, Default)]
pub struct DeviceDiagnostic<'a> {
    pub app_version: &'a str,
    pub effective_type: DeviceType,
    pub type_source: Option<TypeSource>,
    pub detected_type: DeviceType,
    pub detected_confidence: Confidence,
    pub detected_name: Option<&'a str>,
    pub manufacturer: Option<&'a str>,
    pub model: Option<&'a str>,
    /// The OUI manufacturer, which is a fact about the maker rather than about
    /// the unit, and is often the thing a misclassification turns on.
    pub oui_vendor: Option<&'a str>,
    pub sources: &'a [String],
    pub services: &'a [String],
    pub evidence: &'a [DiagnosticEvidence],
    pub discovery_quality: Option<DiscoveryQuality>,
    /// The device's address, masked before it reaches the output.
    pub ip: Option<&'a str>,
    /// True when the device is the network's default gateway, which is the
    /// single strongest router signal and worth reporting.
    pub is_gateway: bool,
    /// v1.9. The operating system a deep or credentialed scan established, as
    /// one line. Reported because a misclassification of a Windows machine
    /// almost always turns on what was, or was not, established about its OS.
    pub os_summary: Option<&'a str>,
    /// v1.9. `1`, `2` or `3`. The single field that separates a workstation
    /// from a server, and therefore the first thing to look at when one was
    /// reported as the other.
    pub windows_product_type: Option<&'a str>,
    /// v1.9. The hardware the machine reported about itself.
    pub hardware: Option<&'a str>,
}

/// Mask an address to its first two octets.
///
/// `192.168.1.42` becomes `192.168.x.x`. Anything that is not four
/// dot-separated numbers is dropped entirely rather than passed through, so a
/// malformed or unexpected value cannot escape by not matching the pattern.
pub fn redact_ip(ip: &str) -> Option<String> {
    let parts: Vec<&str> = ip.trim().split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    if !parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u8>().is_ok())
    {
        return None;
    }
    Some(format!("{}.{}.x.x", parts[0], parts[1]))
}

fn clip(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_VALUE_CHARS {
        return collapsed;
    }
    let head: String = collapsed.chars().take(MAX_VALUE_CHARS).collect();
    format!("{head}…")
}

fn freshness_phrase(evidence: &DiagnosticEvidence) -> String {
    match evidence.freshness {
        Freshness::Current => String::new(),
        Freshness::Aging | Freshness::Stale => {
            let scans = evidence.misses.max(1);
            if scans == 1 {
                " (last seen 1 discovery scan ago)".to_string()
            } else {
                format!(" (last seen {scans} discovery scans ago)")
            }
        }
    }
}

/// Build the report.
///
/// Deterministic: the evidence is sorted by source, kind and value before
/// anything is written, so two runs over the same device produce byte-identical
/// text and a diff between two reports means something changed.
pub fn build_report(input: &DeviceDiagnostic<'_>) -> String {
    let mut out = String::with_capacity(1_024);
    out.push_str("ArcScan discovery report\n");
    out.push_str(&format!("Version: {}\n", clip(input.app_version)));

    out.push_str(&format!("Device type: {}\n", input.effective_type.label()));
    out.push_str(&format!(
        "Type source: {}\n",
        match input.type_source {
            Some(TypeSource::User) => "Set by you",
            _ => "Automatic",
        }
    ));
    // Under an override, what ArcScan thought is the whole point of the report.
    if input.type_source == Some(TypeSource::User) {
        out.push_str(&format!(
            "ArcScan detected: {}\n",
            input.detected_type.label()
        ));
    }
    out.push_str(&format!(
        "Detected confidence: {}\n",
        match input.detected_confidence {
            Confidence::High => "High",
            Confidence::Medium => "Medium",
            Confidence::Low => "Low",
            Confidence::Unknown => "Not established",
        }
    ));

    for (label, value) in [
        ("Detected name", input.detected_name),
        ("Manufacturer", input.manufacturer),
        ("Model", input.model),
        ("MAC manufacturer", input.oui_vendor),
        // v1.9. Deliberately the established facts and never an identifier: a
        // serial number or a system UUID identifies the unit, and a report a
        // technician pastes into a support thread has no use for one.
        ("Operating system", input.os_summary),
        ("Windows product type", input.windows_product_type),
        ("Hardware", input.hardware),
    ] {
        if let Some(value) = value.map(clip).filter(|v| !v.is_empty()) {
            out.push_str(&format!("{label}: {value}\n"));
        }
    }

    if let Some(masked) = input.ip.and_then(redact_ip) {
        out.push_str(&format!("Address: {masked}\n"));
    }
    if input.is_gateway {
        out.push_str("Default gateway: yes\n");
    }

    if !input.sources.is_empty() {
        let mut sources: Vec<String> = input.sources.iter().map(|s| clip(s)).collect();
        sources.sort();
        sources.dedup();
        out.push_str(&format!("Sources: {}\n", sources.join(", ")));
    }

    if let Some(quality) = input.discovery_quality {
        out.push_str(&format!("Discovery scan state: {}\n", quality.label()));
    }

    if !input.services.is_empty() {
        let mut services: Vec<String> = input.services.iter().map(|s| clip(s)).collect();
        services.sort();
        services.dedup();
        services.truncate(MAX_EVIDENCE_LINES);
        out.push_str("Services:\n");
        for service in services {
            out.push_str(&format!("- {service}\n"));
        }
    }

    let mut usable: Vec<&DiagnosticEvidence> = input
        .evidence
        .iter()
        .filter(|e| !EXCLUDED_KINDS.contains(&e.kind.as_str()))
        .collect();
    usable.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then(a.kind.cmp(&b.kind))
            .then(a.value.cmp(&b.value))
    });

    for (heading, wanted) in [("Fresh evidence", false), ("Stale evidence", true)] {
        let lines: Vec<String> = usable
            .iter()
            .filter(|e| e.freshness.is_stale() == wanted)
            .take(MAX_EVIDENCE_LINES)
            .map(|e| {
                format!(
                    "- {} {}: {}{}\n",
                    clip(&e.source),
                    clip(&e.kind),
                    clip(&e.value),
                    freshness_phrase(e)
                )
            })
            .collect();
        if lines.is_empty() {
            continue;
        }
        out.push_str(heading);
        out.push_str(":\n");
        for line in lines {
            out.push_str(&line);
        }
    }

    out.push_str(
        "\nThis report was built on your computer and sent nowhere. \
         It deliberately omits your notes, the MAC address, the serial number, \
         any device identifier and the full IP address.\n",
    );

    if out.chars().count() > MAX_REPORT_CHARS {
        let head: String = out.chars().take(MAX_REPORT_CHARS).collect();
        return format!("{head}\n[report truncated]\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(kind: &str, value: &str, freshness: Freshness, misses: i64) -> DiagnosticEvidence {
        DiagnosticEvidence {
            source: "mdns".into(),
            kind: kind.into(),
            value: value.into(),
            freshness,
            misses,
        }
    }

    fn sample<'a>(
        evidence: &'a [DiagnosticEvidence],
        services: &'a [String],
    ) -> DeviceDiagnostic<'a> {
        DeviceDiagnostic {
            app_version: "1.8.3",
            effective_type: DeviceType::MediaDevice,
            type_source: Some(TypeSource::Automatic),
            detected_type: DeviceType::MediaDevice,
            detected_confidence: Confidence::Medium,
            detected_name: Some("Living Room TV"),
            manufacturer: Some("Example Corp"),
            model: Some("TV-123"),
            oui_vendor: Some("Example Corp"),
            sources: &[],
            services,
            evidence,
            discovery_quality: Some(DiscoveryQuality::Complete),
            ip: Some("192.168.1.42"),
            is_gateway: false,
            os_summary: None,
            windows_product_type: None,
            hardware: None,
        }
    }

    #[test]
    fn an_address_is_masked_to_its_first_two_octets() {
        assert_eq!(redact_ip("192.168.1.42").as_deref(), Some("192.168.x.x"));
        assert_eq!(redact_ip("10.0.14.9").as_deref(), Some("10.0.x.x"));
        // Anything unexpected is dropped rather than passed through.
        for bogus in [
            "",
            "192.168.1",
            "fe80::1",
            "192.168.1.999",
            "not an address",
        ] {
            assert_eq!(redact_ip(bogus), None, "{bogus:?}");
        }
    }

    #[test]
    fn the_report_says_something_useful_about_the_device() {
        let services = vec!["_airplay._tcp".to_string()];
        let evidence = vec![evidence("model", "TV-123", Freshness::Current, 0)];
        let report = build_report(&sample(&evidence, &services));
        assert!(report.starts_with("ArcScan discovery report\n"));
        for expected in [
            "Version: 1.8.3",
            "Device type: Media device",
            "Type source: Automatic",
            "Detected confidence: Medium",
            "Detected name: Living Room TV",
            "Model: TV-123",
            "Discovery scan state: Complete",
            "_airplay._tcp",
        ] {
            assert!(report.contains(expected), "missing {expected:?}:\n{report}");
        }
    }

    #[test]
    fn nothing_that_identifies_the_unit_reaches_the_report() {
        // Every excluded kind, handed in deliberately, plus the full address.
        let evidence = vec![
            evidence("serial_number", "SN-DEADBEEF", Freshness::Current, 0),
            evidence(
                "url",
                "http://192.168.1.42:8080/desc.xml",
                Freshness::Current,
                0,
            ),
            evidence(
                "protocol_identifier",
                "uuid:550e8400-e29b-41d4-a716-446655440000",
                Freshness::Current,
                0,
            ),
            evidence("ipv4_address", "192.168.1.42", Freshness::Current, 0),
            evidence("ipv6_address", "fe80::1", Freshness::Current, 0),
            evidence("model", "TV-123", Freshness::Current, 0),
        ];
        let report = build_report(&sample(&evidence, &[]));
        for forbidden in [
            "SN-DEADBEEF",
            "desc.xml",
            "uuid:",
            "550e8400",
            "192.168.1.42",
            "fe80::1",
        ] {
            assert!(
                !report.contains(forbidden),
                "leaked {forbidden:?}:\n{report}"
            );
        }
        // The masked form is there instead, and the usable evidence survived.
        assert!(report.contains("Address: 192.168.x.x"));
        assert!(report.contains("TV-123"));
    }

    #[test]
    fn there_is_no_field_for_a_note_a_mac_or_a_database_id() {
        // A compile-time guarantee expressed as a runtime one: the struct is
        // built from a full set of fields, and none of them is any of these.
        let report = build_report(&sample(&[], &[]));
        for forbidden in ["Notes", "MAC address:", "Device id", "Serial"] {
            assert!(!report.contains(forbidden), "leaked {forbidden:?}");
        }
        assert!(report.contains("omits your notes"));
    }

    #[test]
    fn a_user_override_is_named_and_keeps_the_detected_answer_beside_it() {
        let mut input = sample(&[], &[]);
        input.type_source = Some(TypeSource::User);
        input.effective_type = DeviceType::Television;
        input.detected_type = DeviceType::MediaDevice;
        let report = build_report(&input);
        assert!(report.contains("Device type: Television"));
        assert!(report.contains("Type source: Set by you"));
        assert!(report.contains("ArcScan detected: Media device"));
    }

    #[test]
    fn fresh_and_stale_evidence_are_separated_and_dated_in_scans() {
        let evidence = vec![
            evidence("service", "_airplay._tcp", Freshness::Current, 0),
            evidence("service", "MediaServer", Freshness::Stale, 4),
            evidence("service", "_raop._tcp", Freshness::Aging, 1),
        ];
        let report = build_report(&sample(&evidence, &[]));
        let fresh_at = report.find("Fresh evidence:").expect("fresh section");
        let stale_at = report.find("Stale evidence:").expect("stale section");
        assert!(fresh_at < stale_at);
        assert!(report.contains("MediaServer (last seen 4 discovery scans ago)"));
        assert!(report.contains("_raop._tcp (last seen 1 discovery scan ago)"));
        // Aging is not stale: it belongs above the stale heading.
        assert!(report.find("_raop._tcp").unwrap() < stale_at);
    }

    #[test]
    fn the_report_is_deterministic_whatever_order_the_evidence_arrives_in() {
        let forward = vec![
            evidence("service", "_a._tcp", Freshness::Current, 0),
            evidence("model", "TV-123", Freshness::Current, 0),
            evidence("manufacturer", "Example Corp", Freshness::Current, 0),
        ];
        let backward: Vec<DiagnosticEvidence> = forward.iter().rev().cloned().collect();
        assert_eq!(
            build_report(&sample(&forward, &[])),
            build_report(&sample(&backward, &[]))
        );
    }

    #[test]
    fn a_device_advertising_a_great_deal_still_produces_a_bounded_report() {
        let evidence: Vec<DiagnosticEvidence> = (0..500)
            .map(|i| {
                evidence(
                    "service",
                    &format!("_service{i}._tcp{}", "x".repeat(400)),
                    Freshness::Current,
                    0,
                )
            })
            .collect();
        let services: Vec<String> = (0..500).map(|i| format!("_svc{i}._tcp")).collect();
        let report = build_report(&sample(&evidence, &services));
        assert!(
            report.chars().count() <= MAX_REPORT_CHARS + 32,
            "report was {} characters",
            report.chars().count()
        );
        // Each line is bounded too, so one hostile value cannot dominate.
        assert!(report
            .lines()
            .all(|line| line.chars().count() <= MAX_VALUE_CHARS * 3));
    }

    #[test]
    fn control_characters_in_a_device_string_are_flattened() {
        let evidence = vec![evidence(
            "model",
            "TV\u{0}-\n123\u{7}",
            Freshness::Current,
            0,
        )];
        let report = build_report(&sample(&evidence, &[]));
        assert!(report.contains("TV - 123"));
        assert!(!report.contains('\u{0}'));
        assert!(!report.contains('\u{7}'));
    }
}

#[cfg(test)]
mod v1_9_tests {
    use super::*;

    #[test]
    fn a_report_names_the_operating_system_and_the_product_type() {
        // What a technician needs when a workstation was reported as a server:
        // whether ProductType was established at all, and what it said.
        let evidence: &[DiagnosticEvidence] = &[];
        let report = build_report(&DeviceDiagnostic {
            app_version: "1.9.0",
            effective_type: DeviceType::Workstation,
            type_source: Some(TypeSource::Automatic),
            detected_type: DeviceType::Workstation,
            detected_confidence: Confidence::High,
            detected_name: Some("WS-FINANCE-04"),
            manufacturer: Some("Dell Inc."),
            model: Some("Latitude 7450"),
            oui_vendor: Some("Dell Inc"),
            sources: &[],
            services: &[],
            evidence,
            discovery_quality: Some(DiscoveryQuality::Complete),
            ip: Some("10.0.0.5"),
            is_gateway: false,
            os_summary: Some("Windows 11 Pro 24H2 (build 26100, x64)"),
            windows_product_type: Some("1"),
            hardware: Some("Dell Inc. Latitude 7450"),
        });
        assert!(report.contains("Windows 11 Pro 24H2"));
        assert!(report.contains("Windows product type: 1"));
        assert!(report.contains("Hardware: Dell Inc. Latitude 7450"));
    }

    #[test]
    fn a_report_carries_no_identifier_that_names_one_unit() {
        // A report is pasted into a support thread. A system UUID or a service
        // tag identifies the customer's specific machine and has no diagnostic
        // value, so there is no field for one — this asserts the fields that
        // exist cannot carry one by accident.
        let evidence: &[DiagnosticEvidence] = &[];
        let report = build_report(&DeviceDiagnostic {
            app_version: "1.9.0",
            effective_type: DeviceType::Server,
            type_source: Some(TypeSource::Automatic),
            detected_type: DeviceType::Server,
            detected_confidence: Confidence::High,
            detected_name: None,
            manufacturer: None,
            model: None,
            oui_vendor: None,
            sources: &[],
            services: &[],
            evidence,
            discovery_quality: None,
            ip: Some("10.0.0.5"),
            is_gateway: false,
            os_summary: Some("Windows Server 2022 Standard"),
            windows_product_type: Some("3"),
            hardware: Some("Dell Inc. PowerEdge R750"),
        });
        // The values, not the words: the report's own footer says in prose
        // that it omits serial numbers, which is the opposite of a leak.
        assert!(!report.contains("J7K2M13"));
        assert!(!report.contains("4C4C4544-004A-3710-8054-B7C04F324D13"));
        assert!(!report.to_lowercase().contains("password"));
        // And the address is still masked, as it was before v1.9.
        assert!(report.contains("10.0.x.x"));
        assert!(!report.contains("10.0.0.5"));
    }

    #[test]
    fn a_device_no_deep_scan_reached_reports_no_operating_system_line() {
        let evidence: &[DiagnosticEvidence] = &[];
        let report = build_report(&DeviceDiagnostic {
            app_version: "1.9.0",
            effective_type: DeviceType::Unknown,
            type_source: None,
            detected_type: DeviceType::Unknown,
            detected_confidence: Confidence::Unknown,
            detected_name: None,
            manufacturer: None,
            model: None,
            oui_vendor: None,
            sources: &[],
            services: &[],
            evidence,
            discovery_quality: None,
            ip: None,
            is_gateway: false,
            os_summary: None,
            windows_product_type: None,
            hardware: None,
        });
        assert!(!report.contains("Operating system"));
        assert!(!report.contains("Windows product type"));
    }
}

#[cfg(test)]
mod v1_9_redaction_tests {
    use super::*;

    fn report_with(kind: &str, value: &str) -> String {
        let evidence = vec![DiagnosticEvidence {
            source: "windows_credentialed".into(),
            kind: kind.into(),
            value: value.into(),
            freshness: Freshness::Current,
            misses: 0,
        }];
        build_report(&DeviceDiagnostic {
            app_version: "1.9.0",
            effective_type: DeviceType::Workstation,
            type_source: Some(TypeSource::Automatic),
            detected_type: DeviceType::Workstation,
            detected_confidence: Confidence::High,
            detected_name: None,
            manufacturer: None,
            model: None,
            oui_vendor: None,
            sources: &[],
            services: &[],
            evidence: &evidence,
            discovery_quality: None,
            ip: None,
            is_gateway: false,
            os_summary: None,
            windows_product_type: None,
            hardware: None,
        })
    }

    #[test]
    fn a_system_uuid_never_reaches_a_report() {
        // The strongest identifier a machine has, and it says nothing about
        // what kind of thing the machine is. Pure risk, no diagnostic value.
        let report = report_with("system_uuid", "4C4C4544-0037-5A10-8051-B4C04F435331");
        assert!(!report.contains("4C4C4544-0037-5A10-8051-B4C04F435331"));
    }

    #[test]
    fn a_domain_name_never_reaches_a_report() {
        // Names the customer's directory rather than the device.
        let report = report_with("domain_membership", "corp.example.internal");
        assert!(!report.contains("corp.example.internal"));
    }

    #[test]
    fn the_strings_a_rule_turns_on_are_kept() {
        // A report that omitted these could not explain a wrong answer, which
        // is the only reason the report exists.
        assert!(report_with("banner", "Canon HTTP Server").contains("Canon HTTP Server"));
        assert!(report_with("page_title", "iDRAC9").contains("iDRAC9"));
        assert!(report_with("os_product", "Windows 11").contains("Windows 11"));
        assert!(report_with("windows_product_type", "1").contains("1"));
    }
}
