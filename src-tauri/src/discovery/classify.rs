//! Deciding what kind of thing a device is, and saying how sure that is.
//!
//! # The shape of the rules
//!
//! Every rule turns observed facts into a claim: a device type, a confidence,
//! and the lines of evidence behind it. All rules are evaluated, every claim is
//! kept, and the strongest one wins. Nothing is thrown away, because a device
//! that looks like two things is worth showing as such — a network-attached
//! printer that also serves a web interface is not a contradiction to hide.
//!
//! # Why confidence is a word and not a number
//!
//! A score invites arithmetic that is not justified: there is no sense in which
//! a printer service is 0.7 of a printer. The four words say what they mean.
//!
//! * **High** — the device declared its own kind through a protocol built for
//!   that purpose, and at least one independent fact agrees.
//! * **Medium** — one protocol-level declaration, with nothing corroborating it.
//! * **Low** — an inference from a port, a manufacturer or a name. Useful to
//!   show, never enough to act on.
//! * **Unknown** — nothing supports any type. Preferred over a guess.
//!
//! # Preferring Unknown
//!
//! A wrong type is worse than no type: it goes in an export, it shapes what an
//! operator looks for, and it is hard to unsee. So a rule only fires on evidence
//! that would convince a technician looking at the same packets.

use std::collections::BTreeSet;

use super::model::{Confidence, DeviceType, DiscoveredDevice, DiscoverySource, EvidenceKind};

/// One type a device might be, with what supports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeClaim {
    pub device_type: DeviceType,
    pub confidence: Confidence,
    /// Plain-language supporting facts, in the order the rule found them.
    pub evidence: Vec<String>,
}

/// What ArcScan settled on, and everything it did not settle on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub device_type: DeviceType,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
    /// Other types with real support, strongest first. Shown in the drawer so a
    /// disagreement is visible rather than silently resolved.
    pub conflicts: Vec<TypeClaim>,
}

impl Default for Classification {
    fn default() -> Self {
        Classification {
            device_type: DeviceType::Unknown,
            confidence: Confidence::Unknown,
            evidence: Vec::new(),
            conflicts: Vec::new(),
        }
    }
}

impl Classification {
    /// A one-line summary for the table and the export, e.g.
    /// `Printer · High confidence`.
    pub fn summary(&self) -> String {
        if self.device_type == DeviceType::Unknown {
            return "Unknown".to_string();
        }
        format!(
            "{} · {} confidence",
            self.device_type.label(),
            self.confidence.as_str()
        )
    }
}

/// Facts from the scan itself, alongside what discovery advertised.
#[derive(Debug, Clone, Default)]
pub struct ClassifyFacts<'a> {
    pub open_ports: &'a [u16],
    pub vendor: Option<&'a str>,
    pub hostname: Option<&'a str>,
    /// True when this address is the network's default gateway.
    pub is_gateway: bool,
    pub os_guess: Option<&'a str>,
}

/// Manufacturer keywords, grouped by what they make.
///
/// Matched as whole words against a lowercased manufacturer string. A maker that
/// builds several categories (Hewlett-Packard makes printers *and* computers;
/// Netgear makes routers *and* storage) appears in each, which is exactly why a
/// manufacturer on its own never produces more than Low confidence.
const PRINTER_MAKERS: &[&str] = &[
    "hp", "hewlett", "brother", "canon", "epson", "lexmark", "xerox", "kyocera", "ricoh", "oki",
    "zebra", "dymo", "sharp", "konica",
];
const TV_MAKERS: &[&str] = &[
    "samsung", "lg", "sony", "vizio", "tcl", "hisense", "philips", "panasonic", "roku", "sceptre",
];
const CAMERA_MAKERS: &[&str] = &[
    "hikvision", "dahua", "axis", "amcrest", "reolink", "wyze", "arlo", "lorex", "foscam",
    "vivotek", "swann",
];
const NAS_MAKERS: &[&str] = &[
    "synology", "qnap", "asustor", "terramaster", "buffalo", "drobo", "western digital", "wd",
];
const NETWORK_MAKERS: &[&str] = &[
    "cisco", "netgear", "tp-link", "tplink", "ubiquiti", "mikrotik", "aruba", "ruckus", "zyxel",
    "d-link", "dlink", "linksys", "eero", "arris", "technicolor", "sagemcom", "actiontec",
    "juniper", "fortinet", "meraki",
];
const COMPUTER_MAKERS: &[&str] = &[
    "apple", "dell", "lenovo", "intel", "micro-star", "asustek", "gigabyte", "acer", "framework",
    "system76", "raspberry",
];
const CONSOLE_MAKERS: &[&str] = &["nintendo", "sony interactive", "microsoft"];
const SPEAKER_MAKERS: &[&str] = &["sonos", "bose", "denon", "yamaha", "marantz", "harman"];
const PHONE_MAKERS: &[&str] = &["apple", "samsung", "google", "oneplus", "xiaomi", "motorola"];

/// True when `haystack` contains `needle` as a whole word.
fn has_word(haystack: &str, needle: &str) -> bool {
    haystack
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == needle)
        || (needle.contains(' ') && haystack.contains(needle))
}

fn made_by(vendor: Option<&str>, makers: &[&str]) -> bool {
    let Some(vendor) = vendor else { return false };
    let lower = vendor.to_lowercase();
    makers.iter().any(|m| has_word(&lower, m))
}

/// Classify one device.
///
/// Deterministic: the inputs are folded into sorted sets before any rule runs,
/// so the order records arrived in cannot change the answer.
pub fn classify(discovery: Option<&DiscoveredDevice>, facts: &ClassifyFacts<'_>) -> Classification {
    let mut services: BTreeSet<String> = BTreeSet::new();
    let mut upnp_types: BTreeSet<String> = BTreeSet::new();
    let mut models: BTreeSet<String> = BTreeSet::new();

    if let Some(d) = discovery {
        for e in d.of_kind(EvidenceKind::Service) {
            services.insert(e.normalized_value.clone());
        }
        for e in d.of_kind(EvidenceKind::DeviceType) {
            if e.source == DiscoverySource::Ssdp {
                upnp_types.insert(e.normalized_value.clone());
            }
        }
        for kind in [
            EvidenceKind::Model,
            EvidenceKind::ModelNumber,
            EvidenceKind::DisplayName,
        ] {
            for e in d.of_kind(kind) {
                models.insert(e.normalized_value.clone());
            }
        }
    }

    let model_text = models.iter().cloned().collect::<Vec<_>>().join(" ");
    let ports: BTreeSet<u16> = facts.open_ports.iter().copied().collect();
    let hostname = facts.hostname.map(str::to_lowercase).unwrap_or_default();

    let has_service = |needle: &str| services.iter().any(|s| s.contains(needle));
    let has_upnp = |needle: &str| upnp_types.iter().any(|t| t.contains(needle));
    let model_says = |needle: &str| model_text.contains(needle);

    let mut claims: Vec<TypeClaim> = Vec::new();
    let mut claim = |device_type: DeviceType, confidence: Confidence, evidence: Vec<String>| {
        claims.push(TypeClaim {
            device_type,
            confidence,
            evidence,
        });
    };

    // ---- Router -----------------------------------------------------------
    if has_upnp("internetgatewaydevice") {
        if facts.is_gateway {
            claim(
                DeviceType::Router,
                Confidence::High,
                vec![
                    "SSDP InternetGatewayDevice".into(),
                    "This network's default gateway".into(),
                ],
            );
        } else {
            claim(
                DeviceType::Router,
                Confidence::Medium,
                vec!["SSDP InternetGatewayDevice".into()],
            );
        }
    } else if facts.is_gateway && made_by(facts.vendor, NETWORK_MAKERS) {
        claim(
            DeviceType::Router,
            Confidence::Medium,
            vec![
                "This network's default gateway".into(),
                format!("{} manufacturer", facts.vendor.unwrap_or_default()),
            ],
        );
    } else if facts.is_gateway {
        claim(
            DeviceType::Router,
            Confidence::Low,
            vec!["This network's default gateway".into()],
        );
    }

    // ---- Printer ----------------------------------------------------------
    let printer_service = has_service("_ipp._tcp")
        || has_service("_ipps._tcp")
        || has_service("_printer._tcp")
        || has_service("_pdl-datastream._tcp")
        || has_upnp("printer");
    let print_port = ports.contains(&9100) || ports.contains(&631) || ports.contains(&515);
    if printer_service && (made_by(facts.vendor, PRINTER_MAKERS) || print_port) {
        let mut evidence = vec![printer_service_label(&services)];
        if let Some(vendor) = facts.vendor.filter(|_| made_by(facts.vendor, PRINTER_MAKERS)) {
            evidence.push(format!("{vendor} manufacturer"));
        }
        if print_port {
            evidence.push(format!("TCP {}", print_ports_label(&ports)));
        }
        claim(DeviceType::Printer, Confidence::High, evidence);
    } else if printer_service {
        claim(
            DeviceType::Printer,
            Confidence::Medium,
            vec![printer_service_label(&services)],
        );
    } else if ports.contains(&9100) {
        claim(
            DeviceType::Printer,
            Confidence::Low,
            vec!["TCP 9100 open, with nothing else to confirm it".into()],
        );
    } else if made_by(facts.vendor, PRINTER_MAKERS) && has_word(&hostname, "printer") {
        claim(
            DeviceType::Printer,
            Confidence::Low,
            vec!["The host name contains \"printer\"".into()],
        );
    }

    // ---- Television and media ---------------------------------------------
    let tv_model = made_by(facts.vendor, TV_MAKERS)
        || model_says("tv")
        || model_says("bravia")
        || model_says("aquos");
    if has_upnp("mediarenderer") && tv_model {
        let mut evidence = vec!["SSDP MediaRenderer".into()];
        if let Some(vendor) = facts.vendor.filter(|_| made_by(facts.vendor, TV_MAKERS)) {
            evidence.push(format!("{vendor} manufacturer"));
        }
        if model_says("tv") {
            evidence.push("The model describes a television".into());
        }
        claim(DeviceType::Television, Confidence::High, evidence);
    } else if has_upnp("mediarenderer") {
        claim(
            DeviceType::MediaDevice,
            Confidence::Medium,
            vec!["SSDP MediaRenderer".into()],
        );
    }

    let cast_service = has_service("_googlecast._tcp");
    let cast_model = model_says("chromecast") || model_says("android tv") || model_says("shield");
    if cast_service && cast_model {
        claim(
            DeviceType::MediaDevice,
            Confidence::High,
            vec![
                "mDNS _googlecast._tcp".into(),
                "The model names a casting device".into(),
            ],
        );
    } else if cast_service {
        claim(
            DeviceType::MediaDevice,
            Confidence::Medium,
            vec!["mDNS _googlecast._tcp".into()],
        );
    }
    if has_service("_airplay._tcp") && !has_service("_workstation._tcp") {
        claim(
            DeviceType::MediaDevice,
            Confidence::Medium,
            vec!["mDNS _airplay._tcp".into()],
        );
    }

    // ---- Speaker ----------------------------------------------------------
    if has_service("_sonos._tcp") || (made_by(facts.vendor, SPEAKER_MAKERS) && has_service("_raop._tcp"))
    {
        claim(
            DeviceType::Speaker,
            Confidence::High,
            vec![
                "An audio-streaming service".into(),
                format!("{} manufacturer", facts.vendor.unwrap_or("Known speaker")),
            ],
        );
    } else if has_service("_raop._tcp") || has_service("_spotify-connect._tcp") {
        claim(
            DeviceType::Speaker,
            Confidence::Medium,
            vec!["An audio-streaming service (AirPlay or Spotify Connect)".into()],
        );
    }

    // ---- Camera -----------------------------------------------------------
    let camera_declared = has_upnp("camera") || has_upnp("digitalsecuritycamera");
    if camera_declared && made_by(facts.vendor, CAMERA_MAKERS) {
        claim(
            DeviceType::Camera,
            Confidence::High,
            vec![
                "SSDP camera device".into(),
                format!("{} manufacturer", facts.vendor.unwrap_or_default()),
            ],
        );
    } else if camera_declared {
        claim(
            DeviceType::Camera,
            Confidence::Medium,
            vec!["SSDP camera device".into()],
        );
    } else if made_by(facts.vendor, CAMERA_MAKERS) && ports.contains(&554) {
        claim(
            DeviceType::Camera,
            Confidence::Medium,
            vec![
                format!("{} manufacturer", facts.vendor.unwrap_or_default()),
                "TCP 554 (RTSP) open".into(),
            ],
        );
    } else if ports.contains(&554) {
        claim(
            DeviceType::Camera,
            Confidence::Low,
            vec!["TCP 554 (RTSP) open, with nothing else to confirm it".into()],
        );
    }

    // ---- Storage ----------------------------------------------------------
    let smb = has_service("_smb._tcp") || ports.contains(&445);
    if smb && made_by(facts.vendor, NAS_MAKERS) {
        claim(
            DeviceType::Nas,
            Confidence::Medium,
            vec![
                "File sharing over SMB".into(),
                format!("{} manufacturer", facts.vendor.unwrap_or_default()),
            ],
        );
    } else if has_upnp("mediaserver") && smb {
        claim(
            DeviceType::Nas,
            Confidence::Medium,
            vec!["SSDP MediaServer".into(), "File sharing over SMB".into()],
        );
    } else if has_upnp("mediaserver") {
        claim(
            DeviceType::MediaDevice,
            Confidence::Medium,
            vec!["SSDP MediaServer".into()],
        );
    }

    // ---- Smart home -------------------------------------------------------
    if has_service("_hap._tcp") || has_service("_homekit._tcp") {
        claim(
            DeviceType::SmartHome,
            Confidence::Medium,
            vec!["mDNS HomeKit accessory protocol".into()],
        );
    } else if has_service("_matter._tcp") || has_service("_matterc._udp") {
        claim(
            DeviceType::SmartHome,
            Confidence::Medium,
            vec!["mDNS Matter commissioning service".into()],
        );
    }

    // ---- Game console -----------------------------------------------------
    if made_by(facts.vendor, CONSOLE_MAKERS) && (model_says("playstation") || model_says("xbox"))
    {
        claim(
            DeviceType::GameConsole,
            Confidence::High,
            vec![
                format!("{} manufacturer", facts.vendor.unwrap_or_default()),
                "The model names a console".into(),
            ],
        );
    } else if model_says("playstation") || model_says("xbox") || model_says("nintendo") {
        claim(
            DeviceType::GameConsole,
            Confidence::Medium,
            vec!["The advertised model names a console".into()],
        );
    }

    // ---- Computers, phones and tablets ------------------------------------
    if has_service("_workstation._tcp") {
        let mut evidence = vec!["mDNS _workstation._tcp".into()];
        let corroborated = has_service("_smb._tcp")
            || has_service("_ssh._tcp")
            || ports.contains(&22)
            || ports.contains(&445)
            || ports.contains(&3389);
        if corroborated {
            evidence.push("An interactive service (SSH, SMB or RDP)".into());
            claim(DeviceType::Computer, Confidence::High, evidence);
        } else {
            claim(DeviceType::Computer, Confidence::Medium, evidence);
        }
    } else if ports.contains(&3389) {
        claim(
            DeviceType::Computer,
            Confidence::Medium,
            vec!["TCP 3389 (Remote Desktop) open".into()],
        );
    } else if made_by(facts.vendor, COMPUTER_MAKERS) && facts.os_guess.is_some() {
        claim(
            DeviceType::Computer,
            Confidence::Low,
            vec![
                format!("{} manufacturer", facts.vendor.unwrap_or_default()),
                format!("TTL suggests {}", facts.os_guess.unwrap_or_default()),
            ],
        );
    } else if ports.contains(&22) {
        claim(
            DeviceType::Computer,
            Confidence::Low,
            vec!["TCP 22 (SSH) open, with nothing else to confirm it".into()],
        );
    }

    if model_says("iphone") || has_word(&hostname, "iphone") {
        claim(
            DeviceType::Phone,
            Confidence::Low,
            vec!["The advertised name suggests a phone".into()],
        );
    } else if model_says("ipad") || has_word(&hostname, "ipad") {
        claim(
            DeviceType::Tablet,
            Confidence::Low,
            vec!["The advertised name suggests a tablet".into()],
        );
    } else if made_by(facts.vendor, PHONE_MAKERS) && has_word(&hostname, "android") {
        claim(
            DeviceType::Phone,
            Confidence::Low,
            vec!["The host name suggests a phone".into()],
        );
    }

    // ---- Network equipment ------------------------------------------------
    if !facts.is_gateway && made_by(facts.vendor, NETWORK_MAKERS) && facts.os_guess.as_deref() == Some("Network device")
    {
        claim(
            DeviceType::NetworkEquipment,
            Confidence::Low,
            vec![
                format!("{} manufacturer", facts.vendor.unwrap_or_default()),
                "TTL suggests network equipment".into(),
            ],
        );
    }

    finish(claims)
}

/// The mDNS or SSDP service that made the printer call, named exactly.
fn printer_service_label(services: &BTreeSet<String>) -> String {
    for name in ["_ipp._tcp", "_ipps._tcp", "_printer._tcp", "_pdl-datastream._tcp"] {
        if services.iter().any(|s| s.contains(name)) {
            return format!("mDNS {name}");
        }
    }
    "An advertised printing service".into()
}

fn print_ports_label(ports: &BTreeSet<u16>) -> String {
    let open: Vec<String> = [631u16, 9100, 515]
        .iter()
        .filter(|p| ports.contains(p))
        .map(|p| p.to_string())
        .collect();
    open.join(" and ")
}

/// Pick the winning claim and file the rest as conflicts.
///
/// Ordering is total and stable: confidence first, then the type's own order,
/// then the evidence text. Two runs over the same facts therefore produce the
/// same winner *and* the same conflict list, which is what keeps a device's type
/// from flickering between scans.
fn finish(mut claims: Vec<TypeClaim>) -> Classification {
    claims.retain(|c| c.device_type != DeviceType::Unknown);
    if claims.is_empty() {
        return Classification::default();
    }
    claims.sort_by(|a, b| {
        a.confidence
            .cmp(&b.confidence)
            .then(a.device_type.cmp(&b.device_type))
            .then(a.evidence.cmp(&b.evidence))
    });
    // Two claims for the same type are one claim; keep the stronger.
    let mut seen: BTreeSet<DeviceType> = BTreeSet::new();
    claims.retain(|c| seen.insert(c.device_type));

    let winner = claims.remove(0);
    Classification {
        device_type: winner.device_type,
        confidence: winner.confidence,
        evidence: winner.evidence,
        conflicts: claims,
    }
}

/// Merge a freshly computed classification with the one already stored.
///
/// A settled, high-confidence type is not given up for a passing low-confidence
/// one: a printer that answered mDNS last week and only ICMP today is still a
/// printer, and letting a quiet scan downgrade it would make the column churn.
/// A new claim that is as strong or stronger always wins, so a genuine change
/// (the address was reassigned to something else) still comes through.
pub fn reconcile(previous: Option<&Classification>, fresh: Classification) -> Classification {
    let Some(previous) = previous else {
        return fresh;
    };
    if previous.device_type == DeviceType::Unknown {
        return fresh;
    }
    if fresh.device_type == DeviceType::Unknown {
        return previous.clone();
    }
    if fresh.confidence.at_least(previous.confidence) {
        fresh
    } else {
        previous.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{Evidence, EvidenceKind};
    use super::*;

    struct Fixture {
        device: DiscoveredDevice,
        ports: Vec<u16>,
        vendor: Option<String>,
        hostname: Option<String>,
        gateway: bool,
        os: Option<String>,
    }

    impl Fixture {
        fn new() -> Self {
            Fixture {
                device: DiscoveredDevice::new("192.0.2.10".parse().unwrap()),
                ports: Vec::new(),
                vendor: None,
                hostname: None,
                gateway: false,
                os: None,
            }
        }
        fn service(mut self, name: &str) -> Self {
            self.device.add(Evidence::new(
                DiscoverySource::Mdns,
                EvidenceKind::Service,
                name,
                name,
                Confidence::High,
            ));
            self
        }
        fn upnp(mut self, kind: &str) -> Self {
            self.device.add(Evidence::new(
                DiscoverySource::Ssdp,
                EvidenceKind::DeviceType,
                "upnp",
                kind,
                Confidence::High,
            ));
            self
        }
        fn model(mut self, model: &str) -> Self {
            self.device.add(Evidence::new(
                DiscoverySource::Ssdp,
                EvidenceKind::Model,
                "",
                model,
                Confidence::High,
            ));
            self
        }
        fn ports(mut self, ports: &[u16]) -> Self {
            self.ports = ports.to_vec();
            self
        }
        fn vendor(mut self, vendor: &str) -> Self {
            self.vendor = Some(vendor.into());
            self
        }
        fn hostname(mut self, hostname: &str) -> Self {
            self.hostname = Some(hostname.into());
            self
        }
        fn gateway(mut self) -> Self {
            self.gateway = true;
            self
        }
        fn os(mut self, os: &str) -> Self {
            self.os = Some(os.into());
            self
        }
        fn run(mut self) -> Classification {
            self.device.sort();
            classify(
                Some(&self.device),
                &ClassifyFacts {
                    open_ports: &self.ports,
                    vendor: self.vendor.as_deref(),
                    hostname: self.hostname.as_deref(),
                    is_gateway: self.gateway,
                    os_guess: self.os.as_deref(),
                },
            )
        }
    }

    #[test]
    fn nothing_at_all_stays_unknown() {
        let c = classify(None, &ClassifyFacts::default());
        assert_eq!(c.device_type, DeviceType::Unknown);
        assert_eq!(c.confidence, Confidence::Unknown);
        assert!(c.evidence.is_empty());
        assert_eq!(c.summary(), "Unknown");
    }

    #[test]
    fn a_gateway_declaring_itself_a_gateway_is_a_router_with_high_confidence() {
        let c = Fixture::new()
            .upnp("InternetGatewayDevice")
            .vendor("Acme Networks")
            .gateway()
            .run();
        assert_eq!(c.device_type, DeviceType::Router);
        assert_eq!(c.confidence, Confidence::High);
        assert!(c.evidence.iter().any(|e| e.contains("InternetGatewayDevice")));
        assert!(c.evidence.iter().any(|e| e.contains("default gateway")));
    }

    #[test]
    fn an_internet_gateway_that_is_not_this_networks_gateway_is_medium() {
        let c = Fixture::new().upnp("InternetGatewayDevice").run();
        assert_eq!(c.device_type, DeviceType::Router);
        assert_eq!(c.confidence, Confidence::Medium);
    }

    #[test]
    fn the_gateway_address_alone_is_only_low_confidence() {
        let c = Fixture::new().gateway().run();
        assert_eq!(c.device_type, DeviceType::Router);
        assert_eq!(c.confidence, Confidence::Low);
    }

    #[test]
    fn a_printer_service_with_a_printer_maker_and_print_ports_is_high() {
        let c = Fixture::new()
            .service("_ipp._tcp.local")
            .vendor("HP Inc.")
            .ports(&[631, 9100])
            .run();
        assert_eq!(c.device_type, DeviceType::Printer);
        assert_eq!(c.confidence, Confidence::High);
        assert!(c.evidence.iter().any(|e| e.contains("_ipp._tcp")));
        assert!(c.evidence.iter().any(|e| e.contains("HP")));
        assert!(c.evidence.iter().any(|e| e.contains("631")));
        assert_eq!(c.summary(), "Printer · high confidence");
    }

    #[test]
    fn a_printer_service_on_its_own_is_medium() {
        let c = Fixture::new().service("_printer._tcp.local").run();
        assert_eq!(c.device_type, DeviceType::Printer);
        assert_eq!(c.confidence, Confidence::Medium);
    }

    #[test]
    fn port_9100_alone_is_low_confidence() {
        let c = Fixture::new().ports(&[9100]).run();
        assert_eq!(c.device_type, DeviceType::Printer);
        assert_eq!(c.confidence, Confidence::Low);
    }

    #[test]
    fn port_22_alone_is_low_confidence() {
        let c = Fixture::new().ports(&[22]).run();
        assert_eq!(c.device_type, DeviceType::Computer);
        assert_eq!(c.confidence, Confidence::Low);
    }

    #[test]
    fn a_manufacturer_alone_never_reaches_medium() {
        for maker in ["HP Inc.", "Synology", "Hikvision", "Sonos", "Cisco"] {
            let c = Fixture::new().vendor(maker).run();
            assert!(
                c.confidence == Confidence::Low || c.confidence == Confidence::Unknown,
                "{maker} produced {:?}",
                c.confidence
            );
        }
    }

    #[test]
    fn a_media_renderer_with_a_television_model_is_a_television() {
        let c = Fixture::new()
            .upnp("MediaRenderer")
            .vendor("Acme")
            .model("Acme 55 inch Smart TV")
            .run();
        assert_eq!(c.device_type, DeviceType::Television);
        assert_eq!(c.confidence, Confidence::High);
    }

    #[test]
    fn a_media_renderer_with_nothing_else_is_a_media_device_at_medium() {
        let c = Fixture::new().upnp("MediaRenderer").run();
        assert_eq!(c.device_type, DeviceType::MediaDevice);
        assert_eq!(c.confidence, Confidence::Medium);
    }

    #[test]
    fn casting_plus_a_casting_model_is_a_media_device_at_high() {
        let c = Fixture::new()
            .service("_googlecast._tcp.local")
            .model("Chromecast Ultra")
            .run();
        assert_eq!(c.device_type, DeviceType::MediaDevice);
        assert_eq!(c.confidence, Confidence::High);
    }

    #[test]
    fn a_declared_camera_from_a_camera_maker_is_high() {
        let c = Fixture::new()
            .upnp("DigitalSecurityCamera")
            .vendor("Hikvision")
            .run();
        assert_eq!(c.device_type, DeviceType::Camera);
        assert_eq!(c.confidence, Confidence::High);
    }

    #[test]
    fn rtsp_alone_is_only_a_low_confidence_camera() {
        let c = Fixture::new().ports(&[554]).run();
        assert_eq!(c.device_type, DeviceType::Camera);
        assert_eq!(c.confidence, Confidence::Low);
    }

    #[test]
    fn smb_from_a_storage_maker_is_a_nas_at_medium() {
        let c = Fixture::new()
            .service("_smb._tcp.local")
            .vendor("Synology")
            .ports(&[445])
            .run();
        assert_eq!(c.device_type, DeviceType::Nas);
        assert_eq!(c.confidence, Confidence::Medium);
    }

    #[test]
    fn a_homekit_accessory_is_a_smart_home_device_at_medium() {
        let c = Fixture::new().service("_hap._tcp.local").run();
        assert_eq!(c.device_type, DeviceType::SmartHome);
        assert_eq!(c.confidence, Confidence::Medium);
    }

    #[test]
    fn a_workstation_with_an_interactive_service_is_a_computer_at_high() {
        let c = Fixture::new()
            .service("_workstation._tcp.local")
            .service("_ssh._tcp.local")
            .ports(&[22])
            .run();
        assert_eq!(c.device_type, DeviceType::Computer);
        assert_eq!(c.confidence, Confidence::High);
    }

    #[test]
    fn a_console_model_from_a_console_maker_is_high() {
        let c = Fixture::new()
            .vendor("Sony Interactive Entertainment")
            .model("PlayStation 5")
            .run();
        assert_eq!(c.device_type, DeviceType::GameConsole);
        assert_eq!(c.confidence, Confidence::High);
    }

    #[test]
    fn a_sonos_speaker_is_recognised() {
        let c = Fixture::new()
            .service("_sonos._tcp.local")
            .vendor("Sonos, Inc.")
            .run();
        assert_eq!(c.device_type, DeviceType::Speaker);
        assert_eq!(c.confidence, Confidence::High);
    }

    #[test]
    fn a_hostname_containing_a_type_word_is_only_low_confidence() {
        let c = Fixture::new().vendor("Brother").hostname("brother-printer").run();
        assert_eq!(c.device_type, DeviceType::Printer);
        assert_eq!(c.confidence, Confidence::Low);
    }

    #[test]
    fn every_type_except_the_placeholders_is_reachable_from_real_evidence() {
        use DeviceType::*;
        let cases: Vec<(DeviceType, Classification)> = vec![
            (Router, Fixture::new().upnp("InternetGatewayDevice").gateway().run()),
            (
                Printer,
                Fixture::new().service("_ipp._tcp.local").ports(&[9100]).run(),
            ),
            (
                Computer,
                Fixture::new().service("_workstation._tcp.local").ports(&[22]).run(),
            ),
            (Phone, Fixture::new().hostname("sams-iphone").run()),
            (Tablet, Fixture::new().hostname("studio-ipad").run()),
            (
                Television,
                Fixture::new().upnp("MediaRenderer").model("Acme Smart TV").run(),
            ),
            (
                MediaDevice,
                Fixture::new().service("_googlecast._tcp.local").run(),
            ),
            (Camera, Fixture::new().upnp("DigitalSecurityCamera").run()),
            (
                Nas,
                Fixture::new().service("_smb._tcp.local").vendor("QNAP").run(),
            ),
            (GameConsole, Fixture::new().model("Xbox Series X").run()),
            (SmartHome, Fixture::new().service("_hap._tcp.local").run()),
            (
                NetworkEquipment,
                Fixture::new().vendor("Ubiquiti").os("Network device").run(),
            ),
            (Speaker, Fixture::new().service("_raop._tcp.local").run()),
        ];
        for (expected, actual) in cases {
            assert_eq!(actual.device_type, expected, "{:?}", actual);
            assert!(!actual.evidence.is_empty(), "{expected:?} has no evidence");
        }
    }

    #[test]
    fn every_confidence_level_is_reachable() {
        let levels: Vec<Confidence> = vec![
            Fixture::new().upnp("InternetGatewayDevice").gateway().run().confidence,
            Fixture::new().upnp("MediaRenderer").run().confidence,
            Fixture::new().ports(&[9100]).run().confidence,
            classify(None, &ClassifyFacts::default()).confidence,
        ];
        assert_eq!(
            levels,
            vec![
                Confidence::High,
                Confidence::Medium,
                Confidence::Low,
                Confidence::Unknown
            ]
        );
    }

    #[test]
    fn a_protocol_declaration_beats_a_port_heuristic() {
        // Port 9100 says printer; the device says it is a camera. The
        // protocol-level declaration is the stronger claim, and the printer
        // reading is kept as a conflict rather than discarded.
        let c = Fixture::new()
            .upnp("DigitalSecurityCamera")
            .vendor("Axis")
            .ports(&[9100])
            .run();
        assert_eq!(c.device_type, DeviceType::Camera);
        assert_eq!(c.confidence, Confidence::High);
        assert!(c
            .conflicts
            .iter()
            .any(|claim| claim.device_type == DeviceType::Printer));
    }

    #[test]
    fn conflicting_evidence_is_kept_and_ordered_by_strength() {
        let c = Fixture::new()
            .service("_ipp._tcp.local")
            .vendor("HP Inc.")
            .ports(&[631, 22])
            .run();
        assert_eq!(c.device_type, DeviceType::Printer);
        assert!(!c.conflicts.is_empty());
        for pair in c.conflicts.windows(2) {
            assert!(pair[0].confidence <= pair[1].confidence);
        }
        // A type never appears twice.
        let mut types: Vec<DeviceType> = c.conflicts.iter().map(|x| x.device_type).collect();
        types.push(c.device_type);
        let unique: BTreeSet<DeviceType> = types.iter().copied().collect();
        assert_eq!(unique.len(), types.len());
    }

    #[test]
    fn the_answer_does_not_depend_on_the_order_evidence_arrived_in() {
        let forward = Fixture::new()
            .service("_ipp._tcp.local")
            .service("_http._tcp.local")
            .upnp("Printer")
            .vendor("Brother")
            .ports(&[631, 9100, 80])
            .run();
        let backward = Fixture::new()
            .upnp("Printer")
            .service("_http._tcp.local")
            .service("_ipp._tcp.local")
            .vendor("Brother")
            .ports(&[80, 9100, 631])
            .run();
        assert_eq!(forward, backward);
    }

    #[test]
    fn a_settled_high_confidence_type_is_not_lost_to_a_quiet_scan() {
        let settled = Fixture::new()
            .service("_ipp._tcp.local")
            .vendor("HP Inc.")
            .ports(&[631])
            .run();
        assert_eq!(settled.confidence, Confidence::High);

        // A later scan sees only an open SSH port.
        let weak = Fixture::new().ports(&[22]).run();
        let merged = reconcile(Some(&settled), weak);
        assert_eq!(merged.device_type, DeviceType::Printer);
        assert_eq!(merged.confidence, Confidence::High);
    }

    #[test]
    fn an_equally_strong_new_reading_does_replace_the_old_one() {
        let old = Fixture::new().upnp("InternetGatewayDevice").gateway().run();
        let new = Fixture::new()
            .service("_ipp._tcp.local")
            .vendor("HP Inc.")
            .ports(&[631])
            .run();
        let merged = reconcile(Some(&old), new);
        assert_eq!(merged.device_type, DeviceType::Printer);
    }

    #[test]
    fn nothing_new_leaves_a_known_type_alone_and_unknown_takes_anything() {
        let known = Fixture::new().upnp("MediaRenderer").run();
        let nothing = classify(None, &ClassifyFacts::default());
        assert_eq!(
            reconcile(Some(&known), nothing.clone()).device_type,
            DeviceType::MediaDevice
        );
        assert_eq!(
            reconcile(Some(&nothing), known.clone()).device_type,
            DeviceType::MediaDevice
        );
        assert_eq!(reconcile(None, known.clone()), known);
    }

    #[test]
    fn manufacturer_matching_is_by_word_not_by_substring() {
        // "lg" must not match inside "Bulging Devices Ltd".
        assert!(!made_by(Some("Bulging Devices Ltd"), TV_MAKERS));
        assert!(made_by(Some("LG Electronics"), TV_MAKERS));
        assert!(made_by(Some("Western Digital Technologies"), NAS_MAKERS));
        assert!(!made_by(None, TV_MAKERS));
    }
}
