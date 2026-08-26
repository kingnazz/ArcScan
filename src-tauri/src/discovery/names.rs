//! Choosing what to call a device, and why.
//!
//! # The rule that matters most
//!
//! A name the operator typed wins. Always, unconditionally, and before any other
//! rule is even consulted. Discovery can add a name where there was none; it can
//! never replace one a person chose.
//!
//! # The order below that
//!
//! 1. a high-confidence mDNS name (the instance label a device publishes)
//! 2. a high-confidence SSDP `friendlyName`
//! 3. the reverse-DNS hostname
//! 4. manufacturer plus a high-confidence device type
//! 5. an mDNS host name
//! 6. the current address
//! 7. the MAC address
//!
//! # Why the order is fixed rather than scored
//!
//! A scoring function that weighs sources against each other produces a name
//! that changes when the weights change, and — worse — a name that can oscillate
//! between two scans because a response arrived in a different order. A fixed
//! order with an explicit generic-name rule is boring, and boring is the point:
//! the same evidence always yields the same name, whatever sequence it arrived
//! in.
//!
//! # Generic names
//!
//! A device that calls itself `printer`, `android` or `device` has told us its
//! category, not its identity, and two of them on one network would read as the
//! same thing. Those names are demoted below the hostname and the
//! manufacturer-and-type form, so they are used only when nothing else exists.

use super::model::{Confidence, DiscoveredDevice, DiscoverySource, EvidenceKind, MAX_FIELD_CHARS};

/// Names that describe a category rather than a device.
///
/// Matched against the whole normalized name, never as a substring: `printer`
/// is generic, `Front Office Printer` is not.
const GENERIC_NAMES: &[&str] = &[
    "device",
    "unknown",
    "localhost",
    "router",
    "gateway",
    "modem",
    "switch",
    "printer",
    "scanner",
    "camera",
    "android",
    "android device",
    "iphone",
    "ipad",
    "computer",
    "pc",
    "desktop",
    "laptop",
    "host",
    "hostname",
    "default",
    "upnp",
    "upnp device",
    "media server",
    "media renderer",
    "nas",
    "server",
    "speaker",
    "tv",
    "smart tv",
    "new device",
    "my device",
    // v1.8.3: model strings vendors ship as a friendly name, which are
    // categories rather than devices in exactly the same way.
    "media player",
    "media device",
    "network device",
    "access point",
    "ip camera",
    "webcam",
    "wireless device",
    "smart device",
    "generic",
    "renderer",
    "player",
    "upnp router",
    "internet gateway device",
    "wfadevice",
];

/// True when a name says what kind of thing it is and nothing more.
pub fn is_generic_name(name: &str) -> bool {
    let normal = name.trim().to_lowercase();
    let normal = normal.trim_end_matches(".local").trim();
    GENERIC_NAMES.contains(&normal)
}

/// True when a string is a protocol identifier wearing a name's clothes.
///
/// A UPnP `friendlyName` is occasionally the UDN, and an mDNS instance label is
/// occasionally a bare GUID. Both are stable, unique and completely
/// unreadable — showing one is strictly worse than showing the address, which
/// at least tells a person where to look.
///
/// Deliberately narrow. `HP LaserJet 400` and `DS923+` are full of digits and
/// must survive untouched, so the rule matches only the two shapes that are
/// unambiguously identifiers: a `uuid:`/`urn:` prefix, and a bare UUID with or
/// without its dashes.
pub fn is_protocol_identifier(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    let lower = lower.trim_end_matches(".local").trim();
    if lower.starts_with("uuid:") || lower.starts_with("urn:") {
        return true;
    }
    let bare = lower.replace('-', "");
    // 32 hex characters, which is a UUID whether or not it was punctuated. A
    // real model number never reaches that length in hex alone.
    bare.len() == 32 && bare.chars().all(|c| c.is_ascii_hexdigit())
}

/// True when a name is the label a device left the factory with.
///
/// `HP-A1B2C3`, `BRW90E2BA`, `Canon_1A2B3C4D`: a maker prefix and a slice of
/// the MAC address. It is not wrong, and it is unique, but it says nothing a
/// person recognises, so it belongs below a reverse-DNS hostname and below the
/// manufacturer-and-type form rather than at the top.
///
/// Demoted, never discarded: it still beats a bare address, and on a network of
/// three identical printers it is the only thing that tells them apart.
pub fn is_factory_default_name(name: &str) -> bool {
    let trimmed = name.trim().trim_end_matches(".local").trim();
    let Some(split) = trimmed.rfind(['-', '_']) else {
        return false;
    };
    let (head, tail) = trimmed.split_at(split);
    let tail = &tail[1..];
    if head.is_empty() || head.len() > 12 || !head.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // Six or more hex digits, and at least one of them a digit — a suffix of
    // pure letters (`Office-Printer`) is a word, not a serial.
    tail.len() >= 6
        && tail.len() <= 12
        && tail.chars().all(|c| c.is_ascii_hexdigit())
        && tail.chars().any(|c| c.is_ascii_digit())
}

/// Cut an mDNS service-instance suffix off a name.
///
/// A PTR target is `Office Printer._ipp._tcp.local`, and the part after the
/// first `._` is the service type, not the device. Only an actual service type
/// is removed — `_word._tcp` or `_word._udp`, optionally followed by
/// `.local` — so a name that merely contains an underscore keeps it.
fn strip_service_instance(name: &str) -> String {
    let trimmed = name.trim_end_matches('.');
    let mut search_from = 0usize;
    while let Some(offset) = trimmed[search_from..].find("._") {
        let at = search_from + offset;
        let rest = trimmed[at + 1..].trim_end_matches('.');
        let rest = rest
            .strip_suffix(".local")
            .or_else(|| rest.strip_suffix(".LOCAL"))
            .unwrap_or(rest);
        let lower = rest.to_lowercase();
        let is_service = (lower.ends_with("._tcp") || lower.ends_with("._udp"))
            && lower.starts_with('_')
            && lower.matches('.').count() == 1;
        if is_service {
            return trimmed[..at].to_string();
        }
        search_from = at + 1;
    }
    trimmed.to_string()
}

/// Tidy a name a device advertised, for display.
///
/// * control characters removed, whitespace collapsed
/// * a trailing `.local` dropped
/// * an immediately repeated word or phrase collapsed, so `HP HP LaserJet` and
///   `Hub 6 Hub 6` read the way their makers meant them to
/// * length capped
///
/// Returns `None` when nothing usable is left.
pub fn tidy_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    // The service-instance suffix goes first: `Office Printer._ipp._tcp.local`
    // has to become `Office Printer`, not `Office Printer._ipp._tcp`.
    let instance = strip_service_instance(collapsed.trim());
    let trimmed = instance
        .trim()
        .trim_end_matches('.')
        .trim_end_matches(".local")
        .trim()
        .to_string();
    if trimmed.is_empty() {
        return None;
    }
    // A raw identifier is refused outright rather than demoted. There is no
    // circumstance in which `uuid:550e8400-...` is the best available name.
    if is_protocol_identifier(&trimmed) {
        return None;
    }
    let deduped = collapse_repeats(&trimmed);
    let capped: String = deduped.chars().take(MAX_FIELD_CHARS).collect();
    let capped = capped.trim().to_string();
    (!capped.is_empty()).then_some(capped)
}

/// Collapse an immediately repeated run of words.
///
/// Vendors do this constantly: the SSDP `friendlyName` is built by concatenating
/// a manufacturer field and a model field that already contains the
/// manufacturer, giving `Acme Acme Hub 6`. Only *adjacent* repeats are removed,
/// so a name that genuinely repeats a word later (`Studio Camera Studio`) is
/// left alone.
fn collapse_repeats(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    // Longest run first, so `A B A B` collapses to `A B` rather than staying put.
    for len in (1..=words.len() / 2).rev() {
        for start in 0..=(words.len() - len * 2) {
            let first = &words[start..start + len];
            let second = &words[start + len..start + len * 2];
            let same = first
                .iter()
                .zip(second.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b));
            if same {
                let mut out: Vec<&str> = Vec::with_capacity(words.len() - len);
                out.extend_from_slice(&words[..start + len]);
                out.extend_from_slice(&words[start + len * 2..]);
                return collapse_repeats(&out.join(" "));
            }
        }
    }
    text.to_string()
}

/// Strip a manufacturer that a model string already repeats, so
/// `Acme` + `Acme LaserFast 400` reads as `Acme LaserFast 400` rather than
/// `Acme Acme LaserFast 400`.
pub fn manufacturer_and_model(manufacturer: Option<&str>, model: Option<&str>) -> Option<String> {
    let model = model.and_then(tidy_name);
    let manufacturer = manufacturer.and_then(tidy_name);
    match (manufacturer, model) {
        (Some(make), Some(model)) => {
            // Not only as a prefix: `Acme` + `LaserFast 400 by Acme` is the
            // same duplication wearing a different hat, and prepending would
            // give `Acme LaserFast 400 by Acme`.
            if contains_word_sequence(&model.to_lowercase(), &make.to_lowercase()) {
                Some(model)
            } else {
                tidy_name(&format!("{make} {model}"))
            }
        }
        (Some(make), None) => Some(make),
        (None, Some(model)) => Some(model),
        (None, None) => None,
    }
}

/// True when `haystack` contains every word of `needle`, in order and adjacent.
///
/// Word-wise rather than by substring, so `Acme` is found inside
/// `Acme LaserFast` but not inside `Acmetronic`, which is a different company.
fn contains_word_sequence(haystack: &str, needle: &str) -> bool {
    let hay: Vec<&str> = haystack.split_whitespace().collect();
    let want: Vec<&str> = needle.split_whitespace().collect();
    if want.is_empty() || want.len() > hay.len() {
        return false;
    }
    hay.windows(want.len())
        .any(|window| window == want.as_slice())
}

/// The name ArcScan settled on, and what it was based on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedName {
    pub name: String,
    pub source: DiscoverySource,
    pub confidence: Confidence,
    /// Other names the device advertised, worth showing but not chosen. Ordered,
    /// de-duplicated, and never containing the chosen name.
    pub alternates: Vec<String>,
}

/// Everything the resolver is allowed to consider, so the rules can be tested
/// without a database row or a scan.
#[derive(Debug, Clone, Default)]
pub struct NameInputs<'a> {
    /// What the operator typed. Wins outright.
    pub custom_name: Option<&'a str>,
    pub hostname: Option<&'a str>,
    pub vendor: Option<&'a str>,
    pub ip: Option<&'a str>,
    pub mac: Option<&'a str>,
    /// The high-confidence device type, when one was established, used for the
    /// manufacturer-and-type form.
    pub type_label: Option<&'a str>,
}

/// Decide what to call a device.
///
/// `discovery` may be `None` — a device with no discovery evidence still gets a
/// name, by exactly the rules that applied before this release.
pub fn resolve(inputs: &NameInputs<'_>, discovery: Option<&DiscoveredDevice>) -> ResolvedName {
    // 1. The operator's name. Nothing below is even looked at.
    if let Some(name) = inputs.custom_name.and_then(tidy_name) {
        return ResolvedName {
            name,
            source: DiscoverySource::User,
            confidence: Confidence::High,
            alternates: discovery.map(collect_alternates).unwrap_or_default(),
        };
    }

    let mut strong: Vec<(DiscoverySource, String)> = Vec::new();
    let mut weak: Vec<(DiscoverySource, String)> = Vec::new();
    let mut mdns_hostname: Option<String> = None;

    if let Some(d) = discovery {
        // Deterministic: evidence is walked in its sorted order and the first
        // high-confidence name from each protocol is the one considered, so two
        // scans that saw the same records agree regardless of packet order.
        for evidence in d.of_kind(EvidenceKind::DisplayName) {
            let Some(name) = tidy_name(&evidence.value) else {
                continue;
            };
            // Generic (`printer`) and factory-default (`HP-A1B2C3`) names are
            // demoted for the same reason: neither identifies the device to the
            // person looking at it. Both stay available further down.
            let usable = evidence.confidence.at_least(Confidence::High)
                && !is_generic_name(&name)
                && !is_factory_default_name(&name);
            let bucket = if usable { &mut strong } else { &mut weak };
            // De-duplicated *per source*, not across them. Two protocols
            // advertising the same name in different casing is the common case,
            // and collapsing them would let whichever sorted first decide the
            // spelling — which is exactly the response-order dependence the
            // fixed priority below exists to avoid.
            if !bucket.iter().any(|(source, existing)| {
                *source == evidence.source && existing.eq_ignore_ascii_case(&name)
            }) {
                bucket.push((evidence.source, name));
            }
        }
        mdns_hostname = d
            .of_kind(EvidenceKind::Hostname)
            .filter(|e| e.source == DiscoverySource::Mdns)
            .find_map(|e| tidy_name(&e.value));
    }

    // 2 and 3: mDNS before SSDP, because an mDNS instance label is chosen by the
    // device's owner far more often than a UPnP friendly name, which is usually
    // the model. Both must be high confidence and non-generic to get here.
    let ordered = [DiscoverySource::Mdns, DiscoverySource::Ssdp];
    for source in ordered {
        if let Some((_, name)) = strong.iter().find(|(s, _)| *s == source) {
            return ResolvedName {
                name: name.clone(),
                source,
                confidence: Confidence::High,
                alternates: alternates_excluding(discovery, name),
            };
        }
    }

    // 4. Reverse DNS. A generic hostname is skipped for the same reason a
    //    generic advertisement is.
    if let Some(hostname) = inputs
        .hostname
        .and_then(tidy_name)
        .filter(|h| !is_generic_name(h) && !is_factory_default_name(h))
    {
        return ResolvedName {
            name: hostname.clone(),
            source: DiscoverySource::ReverseDns,
            confidence: Confidence::Medium,
            alternates: alternates_excluding(discovery, &hostname),
        };
    }

    // 5. Manufacturer plus an established type: "Acme printer" says more than
    //    either half, and more than a bare address.
    if let (Some(vendor), Some(kind)) = (inputs.vendor.and_then(tidy_name), inputs.type_label) {
        if let Some(name) = tidy_name(&format!("{vendor} {}", kind.to_lowercase())) {
            return ResolvedName {
                name: name.clone(),
                source: DiscoverySource::ArpVendor,
                confidence: Confidence::Low,
                alternates: alternates_excluding(discovery, &name),
            };
        }
    }

    // 6. An mDNS host name, then anything the device advertised that was
    //    demoted for being generic or low confidence. Both still beat an
    //    address, because they came from the device itself.
    if let Some(name) = mdns_hostname {
        return ResolvedName {
            name: name.clone(),
            source: DiscoverySource::Mdns,
            confidence: Confidence::Low,
            alternates: alternates_excluding(discovery, &name),
        };
    }
    if let Some((source, name)) = weak.first().cloned() {
        return ResolvedName {
            name: name.clone(),
            source,
            confidence: Confidence::Low,
            alternates: alternates_excluding(discovery, &name),
        };
    }

    // 7 and 8. The manufacturer with an address reads better than a bare
    //    address, which is the behaviour every earlier version had.
    let alternates = discovery.map(collect_alternates).unwrap_or_default();
    if let (Some(vendor), Some(ip)) = (inputs.vendor.and_then(tidy_name), inputs.ip) {
        return ResolvedName {
            name: format!("{vendor} ({ip})"),
            source: DiscoverySource::ArpVendor,
            confidence: Confidence::Unknown,
            alternates,
        };
    }
    if let Some(ip) = inputs.ip.filter(|s| !s.trim().is_empty()) {
        return ResolvedName {
            name: ip.trim().to_string(),
            source: DiscoverySource::ScanObservation,
            confidence: Confidence::Unknown,
            alternates,
        };
    }
    ResolvedName {
        name: inputs
            .mac
            .and_then(tidy_name)
            .unwrap_or_else(|| "Unknown device".to_string()),
        source: DiscoverySource::ScanObservation,
        confidence: Confidence::Unknown,
        alternates,
    }
}

/// Every advertised name, tidied and de-duplicated, in evidence order.
fn collect_alternates(device: &DiscoveredDevice) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for evidence in device.of_kind(EvidenceKind::DisplayName) {
        if let Some(name) = tidy_name(&evidence.value) {
            if !out
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&name))
            {
                out.push(name);
            }
        }
    }
    out
}

fn alternates_excluding(device: Option<&DiscoveredDevice>, chosen: &str) -> Vec<String> {
    device
        .map(collect_alternates)
        .unwrap_or_default()
        .into_iter()
        .filter(|name| !name.eq_ignore_ascii_case(chosen))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::model::Evidence;
    use super::*;

    fn device(names: &[(DiscoverySource, &str, Confidence)]) -> DiscoveredDevice {
        let mut d = DiscoveredDevice::new("192.0.2.10".parse().unwrap());
        for (source, name, confidence) in names {
            d.add(Evidence::new(
                *source,
                EvidenceKind::DisplayName,
                "",
                *name,
                *confidence,
            ));
        }
        d.sort();
        d
    }

    fn inputs<'a>() -> NameInputs<'a> {
        NameInputs {
            ip: Some("192.0.2.10"),
            ..Default::default()
        }
    }

    #[test]
    fn a_user_name_beats_every_discovered_name() {
        let d = device(&[
            (
                DiscoverySource::Mdns,
                "Acme LaserFast 400",
                Confidence::High,
            ),
            (DiscoverySource::Ssdp, "Acme Printer", Confidence::High),
        ]);
        let resolved = resolve(
            &NameInputs {
                custom_name: Some("Front Office Printer"),
                hostname: Some("printer-01"),
                ..inputs()
            },
            Some(&d),
        );
        assert_eq!(resolved.name, "Front Office Printer");
        assert_eq!(resolved.source, DiscoverySource::User);
        // The discovered names are still offered, just not chosen.
        assert_eq!(resolved.alternates.len(), 2);
    }

    #[test]
    fn a_user_name_survives_even_when_discovery_is_stronger_and_newer() {
        let d = device(&[(DiscoverySource::Ssdp, "Brand New Name", Confidence::High)]);
        let resolved = resolve(
            &NameInputs {
                custom_name: Some("  Reception TV  "),
                ..inputs()
            },
            Some(&d),
        );
        assert_eq!(resolved.name, "Reception TV");
    }

    #[test]
    fn mdns_outranks_ssdp_when_both_are_high_confidence() {
        let d = device(&[
            (DiscoverySource::Ssdp, "Acme Model 7", Confidence::High),
            (DiscoverySource::Mdns, "Studio Printer", Confidence::High),
        ]);
        let resolved = resolve(&inputs(), Some(&d));
        assert_eq!(resolved.name, "Studio Printer");
        assert_eq!(resolved.source, DiscoverySource::Mdns);
        assert_eq!(resolved.confidence, Confidence::High);
    }

    #[test]
    fn a_high_confidence_ssdp_name_is_used_when_mdns_has_none() {
        let d = device(&[(DiscoverySource::Ssdp, "Acme Hub 6", Confidence::High)]);
        let resolved = resolve(&inputs(), Some(&d));
        assert_eq!(resolved.name, "Acme Hub 6");
        assert_eq!(resolved.source, DiscoverySource::Ssdp);
    }

    #[test]
    fn reverse_dns_is_used_when_no_strong_advertisement_exists() {
        let resolved = resolve(
            &NameInputs {
                hostname: Some("workshop-mac.lan"),
                ..inputs()
            },
            None,
        );
        assert_eq!(resolved.name, "workshop-mac.lan");
        assert_eq!(resolved.source, DiscoverySource::ReverseDns);
    }

    #[test]
    fn a_generic_advertised_name_is_demoted_below_the_hostname() {
        let d = device(&[(DiscoverySource::Mdns, "printer", Confidence::High)]);
        let resolved = resolve(
            &NameInputs {
                hostname: Some("hp-4th-floor"),
                ..inputs()
            },
            Some(&d),
        );
        assert_eq!(resolved.name, "hp-4th-floor");
        assert_eq!(resolved.source, DiscoverySource::ReverseDns);
    }

    #[test]
    fn a_generic_name_is_still_better_than_a_bare_address() {
        let d = device(&[(DiscoverySource::Ssdp, "UPnP Device", Confidence::High)]);
        let resolved = resolve(&inputs(), Some(&d));
        assert_eq!(resolved.name, "UPnP Device");
        assert_eq!(resolved.confidence, Confidence::Low);
    }

    #[test]
    fn manufacturer_and_type_beat_an_address() {
        let resolved = resolve(
            &NameInputs {
                vendor: Some("Acme Networks"),
                type_label: Some("Printer"),
                ..inputs()
            },
            None,
        );
        assert_eq!(resolved.name, "Acme Networks printer");
    }

    #[test]
    fn an_mdns_hostname_is_used_before_an_address() {
        let mut d = DiscoveredDevice::new("192.0.2.10".parse().unwrap());
        d.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::Hostname,
            "",
            "kitchen-hub.local",
            Confidence::Medium,
        ));
        d.sort();
        let resolved = resolve(&inputs(), Some(&d));
        assert_eq!(resolved.name, "kitchen-hub");
    }

    #[test]
    fn the_fallbacks_match_what_earlier_versions_did() {
        assert_eq!(
            resolve(
                &NameInputs {
                    vendor: Some("Acme"),
                    ..inputs()
                },
                None
            )
            .name,
            "Acme (192.0.2.10)"
        );
        assert_eq!(resolve(&inputs(), None).name, "192.0.2.10");
        assert_eq!(
            resolve(
                &NameInputs {
                    mac: Some("AA:BB:CC:00:11:22"),
                    ip: None,
                    ..Default::default()
                },
                None
            )
            .name,
            "AA:BB:CC:00:11:22"
        );
    }

    #[test]
    fn the_chosen_name_never_appears_in_its_own_alternates() {
        let d = device(&[
            (DiscoverySource::Mdns, "Studio Printer", Confidence::High),
            (DiscoverySource::Ssdp, "studio printer", Confidence::High),
            (DiscoverySource::Ssdp, "Acme LF400", Confidence::Medium),
        ]);
        let resolved = resolve(&inputs(), Some(&d));
        assert_eq!(resolved.name, "Studio Printer");
        assert_eq!(resolved.alternates, vec!["Acme LF400"]);
    }

    #[test]
    fn a_protocol_identifier_is_never_used_as_a_name() {
        for identifier in [
            "uuid:550e8400-e29b-41d4-a716-446655440000",
            "UUID:550E8400-E29B-41D4-A716-446655440000",
            "urn:schemas-upnp-org:device:MediaRenderer:1",
            "550e8400-e29b-41d4-a716-446655440000",
            "550e8400e29b41d4a716446655440000",
            "550e8400-e29b-41d4-a716-446655440000.local",
        ] {
            assert_eq!(tidy_name(identifier), None, "{identifier:?} survived");
            assert!(is_protocol_identifier(identifier), "{identifier:?}");
        }
    }

    #[test]
    fn a_name_full_of_digits_is_left_completely_alone() {
        // The identifier rule is the one most likely to eat a real name, so
        // every shape a real model takes is asserted rather than assumed.
        for good in [
            "HP LaserJet M404",
            "Synology DS923+",
            "Living Room Apple TV",
            "Office Chromecast",
            "AX6000",
            "EPSON ET-2850 Series",
            "Bravia KD-55X80J",
            "192 Studio",
        ] {
            assert_eq!(tidy_name(good).as_deref(), Some(good), "{good:?}");
            assert!(!is_protocol_identifier(good), "{good:?}");
        }
    }

    #[test]
    fn a_service_instance_suffix_is_cut_off_the_instance_label() {
        assert_eq!(
            tidy_name("Office Printer._ipp._tcp.local").as_deref(),
            Some("Office Printer")
        );
        assert_eq!(
            tidy_name("Living Room._airplay._tcp.local.").as_deref(),
            Some("Living Room")
        );
        assert_eq!(
            tidy_name("Kitchen Speaker._raop._udp").as_deref(),
            Some("Kitchen Speaker")
        );
        // An underscore that is not a service type is not a suffix.
        assert_eq!(
            tidy_name("Studio_B Camera").as_deref(),
            Some("Studio_B Camera")
        );
        assert_eq!(tidy_name("Rack._unit_4").as_deref(), Some("Rack._unit_4"));
    }

    #[test]
    fn a_factory_default_label_is_demoted_and_not_discarded() {
        for factory in [
            "HP-A1B2C3",
            "Canon_1A2B3C4D",
            "EPSON-4F2E1A",
            "hp-a1b2c3.local",
        ] {
            assert!(is_factory_default_name(factory), "{factory:?}");
            // Still a usable string; only its ranking changed.
            assert!(tidy_name(factory).is_some(), "{factory:?}");
        }
        for real in [
            "Office-Printer",
            "Front Desk",
            "Rack-Switch",
            "AX-6000",
            "Hub 6",
        ] {
            assert!(!is_factory_default_name(real), "{real:?}");
        }
    }

    #[test]
    fn a_run_together_factory_label_is_deliberately_left_alone() {
        // `BRW90E2BA` really is a Brother printer's factory label, and
        // recognising it would mean splitting a string with no separator in it
        // into a prefix and a hex tail. The same rule reads `RT2600AC` — a
        // Synology router model — as `RT` plus six hex digits, and demoting a
        // real model number is a worse outcome than leaving a serial-looking
        // one at the top. The separator is the evidence; without it there is
        // none, so the rule declines to guess.
        for ambiguous in ["BRW90E2BA", "RT2600AC", "DS1821", "AX6000"] {
            assert!(!is_factory_default_name(ambiguous), "{ambiguous:?}");
        }
    }

    #[test]
    fn a_factory_default_advertisement_loses_to_the_reverse_dns_hostname() {
        let mut device = DiscoveredDevice::new("192.0.2.10".parse().unwrap());
        device.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::DisplayName,
            "",
            "HP-A1B2C3",
            Confidence::High,
        ));
        device.sort();
        let resolved = resolve(
            &NameInputs {
                hostname: Some("frontdesk-printer"),
                ..Default::default()
            },
            Some(&device),
        );
        assert_eq!(resolved.name, "frontdesk-printer");
        assert_eq!(resolved.source, DiscoverySource::ReverseDns);
        // And it is still on file as something the device said.
        assert!(resolved.alternates.iter().any(|a| a == "HP-A1B2C3"));
    }

    #[test]
    fn a_factory_default_name_is_still_used_when_it_is_all_there_is() {
        let mut device = DiscoveredDevice::new("192.0.2.10".parse().unwrap());
        device.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::DisplayName,
            "",
            "HP-A1B2C3",
            Confidence::High,
        ));
        device.sort();
        let resolved = resolve(
            &NameInputs {
                ip: Some("192.0.2.10"),
                ..Default::default()
            },
            Some(&device),
        );
        assert_eq!(resolved.name, "HP-A1B2C3");
    }

    #[test]
    fn a_manufacturer_the_model_mentions_anywhere_is_not_repeated() {
        assert_eq!(
            manufacturer_and_model(Some("Acme"), Some("LaserFast 400 by Acme")).as_deref(),
            Some("LaserFast 400 by Acme")
        );
        assert_eq!(
            manufacturer_and_model(Some("Acme"), Some("LaserFast 400")).as_deref(),
            Some("Acme LaserFast 400")
        );
        // A different company whose name merely starts the same way is not the
        // same company, so its name is still prepended.
        assert_eq!(
            manufacturer_and_model(Some("Acme"), Some("Acmetronic 9")).as_deref(),
            Some("Acme Acmetronic 9")
        );
    }

    #[test]
    fn every_permutation_of_the_same_evidence_yields_the_same_name() {
        // The rule that matters most: a name may not depend on which multicast
        // response happened to arrive first. Asserted over every ordering of a
        // realistically messy set rather than over one swap.
        let claims = [
            Evidence::new(
                DiscoverySource::Ssdp,
                EvidenceKind::DisplayName,
                "",
                "uuid:550e8400-e29b-41d4-a716-446655440000",
                Confidence::High,
            ),
            Evidence::new(
                DiscoverySource::Mdns,
                EvidenceKind::DisplayName,
                "",
                "Living Room TV._airplay._tcp.local",
                Confidence::High,
            ),
            Evidence::new(
                DiscoverySource::Mdns,
                EvidenceKind::DisplayName,
                "alt",
                "LG-4F2E1A",
                Confidence::High,
            ),
            Evidence::new(
                DiscoverySource::Ssdp,
                EvidenceKind::DisplayName,
                "alt",
                "UPnP Device",
                Confidence::Medium,
            ),
            Evidence::new(
                DiscoverySource::Mdns,
                EvidenceKind::Hostname,
                "",
                "lg-4f2e1a.local",
                Confidence::Medium,
            ),
        ];

        let ip: std::net::Ipv4Addr = "192.0.2.10".parse().unwrap();
        let inputs = NameInputs {
            hostname: Some("tv"),
            vendor: Some("LG"),
            ip: Some("192.0.2.10"),
            ..Default::default()
        };

        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for order in permutations(claims.len()) {
            let mut device = DiscoveredDevice::new(ip);
            for index in order {
                device.add(claims[index].clone());
            }
            device.sort();
            let resolved = resolve(&inputs, Some(&device));
            seen.insert(format!("{}|{:?}", resolved.name, resolved.source));
        }
        assert_eq!(seen.len(), 1, "the name depends on packet order: {seen:?}");
        assert!(seen.iter().next().unwrap().starts_with("Living Room TV|"));
    }

    /// Every ordering of `n` indices. `n` stays small on purpose: 120 orderings
    /// is exhaustive proof, 3,628,800 is a slow test that proves the same thing.
    fn permutations(n: usize) -> Vec<Vec<usize>> {
        if n <= 1 {
            return vec![(0..n).collect()];
        }
        let mut out = Vec::new();
        for (index, _) in (0..n).enumerate() {
            for mut rest in permutations(n - 1) {
                for value in rest.iter_mut() {
                    if *value >= index {
                        *value += 1;
                    }
                }
                let mut one = vec![index];
                one.extend(rest);
                out.push(one);
            }
        }
        out
    }

    #[test]
    fn the_result_does_not_depend_on_the_order_evidence_arrived_in() {
        let forward = device(&[
            (DiscoverySource::Ssdp, "Acme Hub 6", Confidence::High),
            (DiscoverySource::Mdns, "Hall Hub", Confidence::High),
            (DiscoverySource::Mdns, "Hall Hub Alt", Confidence::Medium),
        ]);
        let backward = device(&[
            (DiscoverySource::Mdns, "Hall Hub Alt", Confidence::Medium),
            (DiscoverySource::Mdns, "Hall Hub", Confidence::High),
            (DiscoverySource::Ssdp, "Acme Hub 6", Confidence::High),
        ]);
        assert_eq!(
            resolve(&inputs(), Some(&forward)),
            resolve(&inputs(), Some(&backward))
        );
    }

    #[test]
    fn a_weaker_advertisement_cannot_displace_a_stronger_one() {
        let d = device(&[
            (DiscoverySource::Mdns, "Real Name", Confidence::High),
            (DiscoverySource::Mdns, "Guess", Confidence::Low),
            (DiscoverySource::Ssdp, "Another Guess", Confidence::Medium),
        ]);
        assert_eq!(resolve(&inputs(), Some(&d)).name, "Real Name");
    }

    #[test]
    fn names_are_tidied_of_whitespace_control_characters_and_local_suffixes() {
        assert_eq!(
            tidy_name("  Living\tRoom  ").as_deref(),
            Some("Living Room")
        );
        assert_eq!(tidy_name("printer.local").as_deref(), Some("printer"));
        assert_eq!(tidy_name("printer.local.").as_deref(), Some("printer"));
        assert_eq!(tidy_name("Bad\u{0}Name").as_deref(), Some("Bad Name"));
        assert_eq!(tidy_name("   "), None);
        assert_eq!(tidy_name(""), None);
        assert_eq!(
            tidy_name(&"x".repeat(MAX_FIELD_CHARS * 3))
                .unwrap()
                .chars()
                .count(),
            MAX_FIELD_CHARS
        );
    }

    #[test]
    fn adjacent_repeats_collapse_and_distant_ones_do_not() {
        assert_eq!(tidy_name("HP HP LaserJet").as_deref(), Some("HP LaserJet"));
        assert_eq!(tidy_name("Hub 6 Hub 6").as_deref(), Some("Hub 6"));
        assert_eq!(
            tidy_name("Acme acme Camera").as_deref(),
            Some("Acme Camera")
        );
        // A genuine repeat that is not adjacent stays, because it is probably
        // what the owner meant.
        assert_eq!(
            tidy_name("Studio Camera Studio").as_deref(),
            Some("Studio Camera Studio")
        );
    }

    #[test]
    fn a_manufacturer_already_in_the_model_is_not_repeated() {
        assert_eq!(
            manufacturer_and_model(Some("Acme"), Some("Acme LaserFast 400")).as_deref(),
            Some("Acme LaserFast 400")
        );
        assert_eq!(
            manufacturer_and_model(Some("Acme"), Some("LaserFast 400")).as_deref(),
            Some("Acme LaserFast 400")
        );
        assert_eq!(
            manufacturer_and_model(Some("Acme"), None).as_deref(),
            Some("Acme")
        );
        assert_eq!(
            manufacturer_and_model(None, Some("LF400")).as_deref(),
            Some("LF400")
        );
        assert_eq!(manufacturer_and_model(None, None), None);
    }

    #[test]
    fn generic_names_are_recognised_whole_never_as_substrings() {
        for generic in [
            "printer",
            "PRINTER",
            "  router  ",
            "android",
            "printer.local",
        ] {
            assert!(is_generic_name(generic), "{generic} should be generic");
        }
        for specific in [
            "Front Office Printer",
            "router-2",
            "Android of Sam",
            "Camera 4",
        ] {
            assert!(!is_generic_name(specific), "{specific} should be specific");
        }
    }
}
