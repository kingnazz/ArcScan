//! The shared vocabulary of local discovery: where a fact came from, what kind
//! of fact it is, how much it is worth, and what one device's collected facts
//! look like once mDNS and SSDP have both had their say.
//!
//! Nothing here talks to a socket. Keeping the data model pure is what lets the
//! merge, naming and classification rules be tested against fixtures rather than
//! against whatever happens to be plugged in.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};

/// Where a piece of information about a device came from.
///
/// The order of the variants is deliberate and load-bearing: it is the tie-break
/// used whenever two sources claim the same kind of fact, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySource {
    /// A name the operator typed. Never produced by discovery, only compared
    /// against, and it outranks everything.
    User,
    /// A UPnP device description document fetched from a validated local URL.
    Ssdp,
    /// A multicast DNS record.
    Mdns,
    /// A PTR record from the system resolver.
    ReverseDns,
    /// The OUI table applied to an observed MAC address.
    ArpVendor,
    /// An open TCP port from the scan itself.
    TcpService,
    /// Anything else the scan observed directly.
    ScanObservation,
}

impl DiscoverySource {
    pub fn as_str(self) -> &'static str {
        match self {
            DiscoverySource::User => "user",
            DiscoverySource::Ssdp => "ssdp",
            DiscoverySource::Mdns => "mdns",
            DiscoverySource::ReverseDns => "reverse_dns",
            DiscoverySource::ArpVendor => "arp_vendor",
            DiscoverySource::TcpService => "tcp_service",
            DiscoverySource::ScanObservation => "scan_observation",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user" => DiscoverySource::User,
            "ssdp" => DiscoverySource::Ssdp,
            "mdns" => DiscoverySource::Mdns,
            "reverse_dns" => DiscoverySource::ReverseDns,
            "arp_vendor" => DiscoverySource::ArpVendor,
            "tcp_service" => DiscoverySource::TcpService,
            "scan_observation" => DiscoverySource::ScanObservation,
            _ => return None,
        })
    }
}

/// What a piece of evidence is *about*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    DisplayName,
    Hostname,
    Manufacturer,
    Model,
    ModelNumber,
    SerialNumber,
    DeviceType,
    Service,
    ServicePort,
    Url,
    Ipv4Address,
    Ipv6Address,
    /// A protocol-level identifier such as a UPnP UDN or an mDNS instance name.
    /// Recorded for continuity checks, never used as a device identity key.
    ProtocolIdentifier,
}

impl EvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::DisplayName => "display_name",
            EvidenceKind::Hostname => "hostname",
            EvidenceKind::Manufacturer => "manufacturer",
            EvidenceKind::Model => "model",
            EvidenceKind::ModelNumber => "model_number",
            EvidenceKind::SerialNumber => "serial_number",
            EvidenceKind::DeviceType => "device_type",
            EvidenceKind::Service => "service",
            EvidenceKind::ServicePort => "service_port",
            EvidenceKind::Url => "url",
            EvidenceKind::Ipv4Address => "ipv4_address",
            EvidenceKind::Ipv6Address => "ipv6_address",
            EvidenceKind::ProtocolIdentifier => "protocol_identifier",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "display_name" => EvidenceKind::DisplayName,
            "hostname" => EvidenceKind::Hostname,
            "manufacturer" => EvidenceKind::Manufacturer,
            "model" => EvidenceKind::Model,
            "model_number" => EvidenceKind::ModelNumber,
            "serial_number" => EvidenceKind::SerialNumber,
            "device_type" => EvidenceKind::DeviceType,
            "service" => EvidenceKind::Service,
            "service_port" => EvidenceKind::ServicePort,
            "url" => EvidenceKind::Url,
            "ipv4_address" => EvidenceKind::Ipv4Address,
            "ipv6_address" => EvidenceKind::Ipv6Address,
            "protocol_identifier" => EvidenceKind::ProtocolIdentifier,
            _ => return None,
        })
    }
}

/// How much weight a single claim carries.
///
/// Deliberately four words rather than a number: a percentage invites a reader
/// to believe the difference between 71% and 68% means something, and it does
/// not. The ordering is strongest-first so `min`/`max` read naturally.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
    /// The default on purpose: nothing known is never quietly upgraded to
    /// something known.
    #[default]
    Unknown,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
            Confidence::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "high" => Confidence::High,
            "medium" => Confidence::Medium,
            "low" => Confidence::Low,
            _ => Confidence::Unknown,
        }
    }

    /// True when this confidence is at least as strong as `other`.
    pub fn at_least(self, other: Confidence) -> bool {
        self <= other
    }
}

/// What ArcScan is prepared to call a device.
///
/// The list is short on purpose. Every entry has to be reachable from evidence a
/// read-only local scan can actually collect; a category nothing can ever prove
/// is worse than Unknown, because it looks like an answer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Router,
    Printer,
    Computer,
    Phone,
    Tablet,
    Television,
    MediaDevice,
    Camera,
    Nas,
    GameConsole,
    SmartHome,
    NetworkEquipment,
    Speaker,
    /// The default, and the answer preferred over an unsupported guess.
    #[default]
    Unknown,
}

impl DeviceType {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceType::Router => "router",
            DeviceType::Printer => "printer",
            DeviceType::Computer => "computer",
            DeviceType::Phone => "phone",
            DeviceType::Tablet => "tablet",
            DeviceType::Television => "television",
            DeviceType::MediaDevice => "media_device",
            DeviceType::Camera => "camera",
            DeviceType::Nas => "nas",
            DeviceType::GameConsole => "game_console",
            DeviceType::SmartHome => "smart_home",
            DeviceType::NetworkEquipment => "network_equipment",
            DeviceType::Speaker => "speaker",
            DeviceType::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "router" => DeviceType::Router,
            "printer" => DeviceType::Printer,
            "computer" => DeviceType::Computer,
            "phone" => DeviceType::Phone,
            "tablet" => DeviceType::Tablet,
            "television" => DeviceType::Television,
            "media_device" => DeviceType::MediaDevice,
            "camera" => DeviceType::Camera,
            "nas" => DeviceType::Nas,
            "game_console" => DeviceType::GameConsole,
            "smart_home" => DeviceType::SmartHome,
            "network_equipment" => DeviceType::NetworkEquipment,
            "speaker" => DeviceType::Speaker,
            _ => DeviceType::Unknown,
        }
    }

    /// The words the interface shows. Kept beside the wire values so a rename
    /// cannot leave the two out of step.
    pub fn label(self) -> &'static str {
        match self {
            DeviceType::Router => "Router",
            DeviceType::Printer => "Printer",
            DeviceType::Computer => "Computer",
            DeviceType::Phone => "Phone",
            DeviceType::Tablet => "Tablet",
            DeviceType::Television => "Television",
            DeviceType::MediaDevice => "Media device",
            DeviceType::Camera => "Camera",
            DeviceType::Nas => "NAS",
            DeviceType::GameConsole => "Game console",
            DeviceType::SmartHome => "Smart-home device",
            DeviceType::NetworkEquipment => "Network equipment",
            DeviceType::Speaker => "Speaker",
            DeviceType::Unknown => "Unknown",
        }
    }

    /// Parse a type, refusing anything that is not one of the words above.
    ///
    /// [`DeviceType::parse`] is deliberately forgiving: a value written by a
    /// newer build has to render as *something*, and Unknown is the honest
    /// answer. That is exactly the wrong behaviour at a trust boundary — an
    /// operator's type override arrives from the interface and is stored, so a
    /// typo or a tampered payload must be refused rather than quietly recorded
    /// as an explicit choice of Unknown, which is itself a meaningful answer.
    pub fn parse_strict(s: &str) -> Option<Self> {
        DeviceType::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Every type, for exhaustive tests and for the interface's filter list.
    pub const ALL: [DeviceType; 14] = [
        DeviceType::Router,
        DeviceType::Printer,
        DeviceType::Computer,
        DeviceType::Phone,
        DeviceType::Tablet,
        DeviceType::Television,
        DeviceType::MediaDevice,
        DeviceType::Camera,
        DeviceType::Nas,
        DeviceType::GameConsole,
        DeviceType::SmartHome,
        DeviceType::NetworkEquipment,
        DeviceType::Speaker,
        DeviceType::Unknown,
    ];
}

/// One claim about one device, from one source.
///
/// `value` is what the device actually said, kept for display.
/// `normalized_value` is what comparison and uniqueness use, so a device that
/// re-advertises the same fact with different spacing or casing does not grow a
/// second row every scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub source: DiscoverySource,
    pub kind: EvidenceKind,
    /// Discriminator within a kind, e.g. the service type for a `service`.
    /// Empty when the kind is single-valued.
    pub key: String,
    pub value: String,
    pub normalized_value: String,
    pub confidence: Confidence,
    /// Small structured extras (a port, a TXT subset), already bounded by the
    /// parser. Never a raw packet and never a whole XML document.
    pub metadata: BTreeMap<String, String>,
}

impl Evidence {
    pub fn new(
        source: DiscoverySource,
        kind: EvidenceKind,
        key: impl Into<String>,
        value: impl Into<String>,
        confidence: Confidence,
    ) -> Self {
        let value: String = value.into();
        Evidence {
            source,
            kind,
            key: key.into(),
            normalized_value: normalize_value(&value),
            value,
            confidence,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_meta(mut self, key: &str, value: impl Into<String>) -> Self {
        self.metadata.insert(key.to_string(), value.into());
        self
    }

    /// The identity used for upsert: two observations that agree on all of this
    /// are the same claim seen twice, not two claims.
    pub fn dedupe_key(&self) -> (DiscoverySource, EvidenceKind, &str, &str) {
        (
            self.source,
            self.kind,
            self.key.as_str(),
            self.normalized_value.as_str(),
        )
    }
}

/// Fold a device-supplied string into a comparable form: trimmed, collapsed
/// whitespace, case-folded. Control characters are dropped rather than escaped,
/// because they carry no meaning here and a stray one would otherwise make the
/// same value look new on every scan.
pub fn normalize_value(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The maximum length of any device-supplied string ArcScan keeps.
///
/// Applied at the parser, so nothing longer ever reaches the database, the
/// interface or an export. Long enough for a real friendly name and a real model
/// string together; short enough that a device advertising a megabyte of text
/// costs nothing.
pub const MAX_FIELD_CHARS: usize = 128;

/// Trim, strip control characters, collapse whitespace and cap the length of a
/// value taken from the network. Returns `None` for anything left empty.
pub fn sanitize_field(value: &str) -> Option<String> {
    let cleaned: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    // Cap by characters rather than bytes so a multi-byte name is never cut
    // through the middle of a code point.
    let capped: String = collapsed.chars().take(MAX_FIELD_CHARS).collect();
    Some(capped)
}

/// Everything discovery learned about one address during one scan.
///
/// Keyed by IPv4 because that is the only thing both protocols reliably agree
/// on and the only thing that maps onto a scanned host. An mDNS instance name or
/// a UPnP UDN is *evidence attached to* this record, never the key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub ipv4: Option<Ipv4Addr>,
    /// Addresses the device advertised over mDNS. Shown as supplemental
    /// information only; ArcScan does not scan IPv6.
    pub ipv6: BTreeSet<Ipv6Addr>,
    /// Every claim collected, in a deterministic order.
    pub evidence: Vec<Evidence>,
    /// Which protocols contributed anything at all.
    pub sources: BTreeSet<DiscoverySource>,
}

impl DiscoveredDevice {
    pub fn new(ipv4: Ipv4Addr) -> Self {
        DiscoveredDevice {
            ipv4: Some(ipv4),
            ..Default::default()
        }
    }

    /// Add a claim, keeping the strongest confidence when the same claim arrives
    /// twice. Order of arrival can therefore never change the result.
    pub fn add(&mut self, evidence: Evidence) {
        self.sources.insert(evidence.source);
        if let Some(existing) = self
            .evidence
            .iter_mut()
            .find(|e| e.dedupe_key() == evidence.dedupe_key())
        {
            if evidence.confidence < existing.confidence {
                existing.confidence = evidence.confidence;
            }
            for (k, v) in evidence.metadata {
                existing.metadata.entry(k).or_insert(v);
            }
            return;
        }
        self.evidence.push(evidence);
    }

    /// Sort the collected evidence into a stable order, so two scans that saw
    /// the same facts in a different order produce byte-identical records.
    pub fn sort(&mut self) {
        self.evidence.sort_by(|a, b| {
            a.kind
                .cmp(&b.kind)
                .then(a.confidence.cmp(&b.confidence))
                .then(a.source.cmp(&b.source))
                .then(a.key.cmp(&b.key))
                .then(a.normalized_value.cmp(&b.normalized_value))
        });
    }

    pub fn best(&self, kind: EvidenceKind) -> Option<&Evidence> {
        self.evidence
            .iter()
            .filter(|e| e.kind == kind)
            .min_by(|a, b| {
                a.confidence
                    .cmp(&b.confidence)
                    .then(a.source.cmp(&b.source))
                    .then(a.normalized_value.cmp(&b.normalized_value))
            })
    }

    pub fn of_kind(&self, kind: EvidenceKind) -> impl Iterator<Item = &Evidence> {
        self.evidence.iter().filter(move |e| e.kind == kind)
    }

    /// The advertised service types, normalized and de-duplicated.
    pub fn services(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .of_kind(EvidenceKind::Service)
            .map(|e| e.value.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty() && self.ipv6.is_empty()
    }
}

/// How much discovery a scan actually managed to do.
///
/// Recorded with the scan so History can explain itself, and — more
/// importantly — so a later comparison can refuse to draw conclusions from two
/// scans with different discovery capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMode {
    /// Both protocols ran to completion.
    Full,
    /// At least one protocol ran, but the scan was stopped or a protocol failed.
    Partial,
    /// Discovery did not run: remote target, switched off, or no local
    /// interface to send from.
    None,
}

impl DiscoveryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DiscoveryMode::Full => "full",
            DiscoveryMode::Partial => "partial",
            DiscoveryMode::None => "none",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "full" => DiscoveryMode::Full,
            "partial" => DiscoveryMode::Partial,
            _ => DiscoveryMode::None,
        }
    }
}

/// What one scan's discovery pass did, for History and for comparison
/// compatibility. Counts only — no addresses, no names, no packets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryReport {
    #[serde(default)]
    pub mdns_attempted: bool,
    #[serde(default)]
    pub ssdp_attempted: bool,
    #[serde(default)]
    pub mdns_responses: usize,
    #[serde(default)]
    pub ssdp_responses: usize,
    #[serde(default)]
    pub descriptions_fetched: usize,
    #[serde(default)]
    pub descriptions_rejected: usize,
    /// Why description URLs were refused or failed, de-duplicated. Reasons
    /// only — never the URL itself, which is a device-supplied string that has
    /// no business being stored or shown back.
    #[serde(default)]
    pub description_notes: Vec<String>,
    #[serde(default)]
    pub devices_enriched: usize,
    #[serde(default)]
    pub duration_ms: u64,
    /// True when the mDNS socket could not be opened or bound.
    ///
    /// Distinct from `mdns_attempted`, which only says the protocol was
    /// switched on. A firewall or a busy interface turns an intended pass into
    /// one that heard nothing, and History has to be able to tell that apart
    /// from a quiet network.
    #[serde(default)]
    pub mdns_socket_failed: bool,
    #[serde(default)]
    pub ssdp_socket_failed: bool,
    /// True when a response cap was reached, so the pass stopped listening
    /// while the link was still talking.
    #[serde(default)]
    pub mdns_capped: bool,
    #[serde(default)]
    pub ssdp_capped: bool,
    /// True when the description budget ran out with documents still queued.
    #[serde(default)]
    pub descriptions_capped: bool,
    /// Why nothing ran, in plain words, when nothing ran.
    #[serde(default)]
    pub skip_reason: Option<String>,
    /// True when Stop landed during discovery.
    #[serde(default)]
    pub interrupted: bool,
}

impl DiscoveryReport {
    pub fn skipped(reason: impl Into<String>) -> Self {
        DiscoveryReport {
            skip_reason: Some(reason.into()),
            ..Default::default()
        }
    }

    /// The mode this report represents.
    ///
    /// `Full` requires both protocols to have been attempted and the pass to
    /// have finished on its own terms. That is a deliberately strict bar: it is
    /// the gate for recording discovery change events, and a scan that was cut
    /// short must never make a service look like it went away.
    pub fn mode(&self) -> DiscoveryMode {
        if !self.mdns_attempted && !self.ssdp_attempted {
            return DiscoveryMode::None;
        }
        if self.interrupted || !self.mdns_attempted || !self.ssdp_attempted {
            return DiscoveryMode::Partial;
        }
        DiscoveryMode::Full
    }

    /// How well the discovery pass went, in the four words History shows.
    ///
    /// Deliberately narrower than a guess: ArcScan never says a firewall
    /// blocked anything, because it cannot observe that. It says the socket
    /// failed, or a cap was reached, or a description was refused — each of
    /// which it did observe — and leaves the diagnosis to the person, who can
    /// see their own firewall and ArcScan cannot.
    pub fn quality(&self) -> DiscoveryQuality {
        if !self.mdns_attempted && !self.ssdp_attempted {
            return DiscoveryQuality::Skipped;
        }
        if self.interrupted {
            return DiscoveryQuality::Interrupted;
        }
        let limited = !self.mdns_attempted
            || !self.ssdp_attempted
            || self.mdns_socket_failed
            || self.ssdp_socket_failed
            || self.mdns_capped
            || self.ssdp_capped
            || self.descriptions_capped
            || self.descriptions_rejected > 0;
        if limited {
            DiscoveryQuality::Limited
        } else {
            DiscoveryQuality::Complete
        }
    }

    /// Why the pass was less than complete, in one short phrase, or `None`
    /// when it was complete.
    ///
    /// One reason, not a list: History has a line, not a paragraph, and the
    /// full detail is already in the scan's stored summary.
    pub fn quality_reason(&self) -> Option<&'static str> {
        match self.quality() {
            DiscoveryQuality::Complete => None,
            DiscoveryQuality::Skipped => Some("Not run"),
            DiscoveryQuality::Interrupted => Some("Scan stopped"),
            DiscoveryQuality::Limited => {
                Some(if self.mdns_socket_failed && self.ssdp_socket_failed {
                    "No discovery socket"
                } else if self.mdns_socket_failed {
                    "mDNS socket unavailable"
                } else if self.ssdp_socket_failed {
                    "SSDP socket unavailable"
                } else if !self.mdns_attempted {
                    "mDNS not run"
                } else if !self.ssdp_attempted {
                    "SSDP not run"
                } else if self.mdns_capped || self.ssdp_capped {
                    "Response limit reached"
                } else if self.descriptions_capped {
                    "Description limit reached"
                } else {
                    "Some descriptions refused"
                })
            }
        }
    }
}

/// How well one scan's discovery pass went.
///
/// Separate from [`DiscoveryMode`], which answers a different question.
/// `DiscoveryMode` gates *comparison* — whether two scans may be reasoned about
/// together — and its rules must not move, because changing them would change
/// which change events a database produces. `DiscoveryQuality` is for a person
/// reading History, and can be as descriptive as the evidence supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryQuality {
    /// Both protocols ran, the pass finished, and nothing was cut short.
    Complete,
    /// Discovery ran but could not do all of it: a socket failed, a cap was
    /// reached, or a description was refused.
    Limited,
    /// Discovery did not run at all: a remote target, switched off, or no
    /// eligible interface.
    Skipped,
    /// Stop landed while discovery was in progress.
    Interrupted,
}

impl DiscoveryQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            DiscoveryQuality::Complete => "complete",
            DiscoveryQuality::Limited => "limited",
            DiscoveryQuality::Skipped => "skipped",
            DiscoveryQuality::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "complete" => DiscoveryQuality::Complete,
            "limited" => DiscoveryQuality::Limited,
            "interrupted" => DiscoveryQuality::Interrupted,
            _ => DiscoveryQuality::Skipped,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DiscoveryQuality::Complete => "Complete",
            DiscoveryQuality::Limited => "Limited",
            DiscoveryQuality::Skipped => "Skipped",
            DiscoveryQuality::Interrupted => "Interrupted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_kind_round_trip_through_their_wire_names() {
        for source in [
            DiscoverySource::User,
            DiscoverySource::Ssdp,
            DiscoverySource::Mdns,
            DiscoverySource::ReverseDns,
            DiscoverySource::ArpVendor,
            DiscoverySource::TcpService,
            DiscoverySource::ScanObservation,
        ] {
            assert_eq!(DiscoverySource::parse(source.as_str()), Some(source));
        }
        assert_eq!(DiscoverySource::parse("telepathy"), None);

        for kind in [
            EvidenceKind::DisplayName,
            EvidenceKind::Hostname,
            EvidenceKind::Manufacturer,
            EvidenceKind::Model,
            EvidenceKind::ModelNumber,
            EvidenceKind::SerialNumber,
            EvidenceKind::DeviceType,
            EvidenceKind::Service,
            EvidenceKind::ServicePort,
            EvidenceKind::Url,
            EvidenceKind::Ipv4Address,
            EvidenceKind::Ipv6Address,
            EvidenceKind::ProtocolIdentifier,
        ] {
            assert_eq!(EvidenceKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(EvidenceKind::parse("vibes"), None);
    }

    #[test]
    fn every_device_type_round_trips_and_has_a_label() {
        for kind in DeviceType::ALL {
            assert_eq!(DeviceType::parse(kind.as_str()), kind);
            assert!(!kind.label().is_empty());
        }
        // An unrecognised value from a newer build reads as Unknown rather than
        // making the row unrenderable.
        assert_eq!(DeviceType::parse("toaster"), DeviceType::Unknown);
    }

    #[test]
    fn confidence_orders_strongest_first() {
        assert!(Confidence::High < Confidence::Medium);
        assert!(Confidence::Medium < Confidence::Low);
        assert!(Confidence::Low < Confidence::Unknown);
        assert!(Confidence::High.at_least(Confidence::Medium));
        assert!(!Confidence::Low.at_least(Confidence::High));
        for c in [
            Confidence::High,
            Confidence::Medium,
            Confidence::Low,
            Confidence::Unknown,
        ] {
            assert_eq!(Confidence::parse(c.as_str()), c);
        }
    }

    #[test]
    fn normalization_folds_case_whitespace_and_control_characters() {
        assert_eq!(normalize_value("  HP  LaserJet\tPro "), "hp laserjet pro");
        assert_eq!(normalize_value("Living\u{0}Room"), "living room");
        assert_eq!(normalize_value("\n\n"), "");
    }

    #[test]
    fn sanitize_caps_length_without_splitting_a_character() {
        let long = "é".repeat(MAX_FIELD_CHARS * 4);
        let capped = sanitize_field(&long).unwrap();
        assert_eq!(capped.chars().count(), MAX_FIELD_CHARS);
        assert!(capped.chars().all(|c| c == 'é'));
        assert_eq!(sanitize_field("   \t  "), None);
        assert_eq!(sanitize_field(" a\u{7}b "), Some("a b".to_string()));
    }

    #[test]
    fn the_same_claim_twice_is_one_row_and_keeps_the_stronger_confidence() {
        let mut device = DiscoveredDevice::new("192.0.2.10".parse().unwrap());
        device.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::DisplayName,
            "",
            "Office Printer",
            Confidence::Medium,
        ));
        device.add(Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::DisplayName,
            "",
            "office   printer",
            Confidence::High,
        ));
        assert_eq!(device.evidence.len(), 1);
        assert_eq!(device.evidence[0].confidence, Confidence::High);
        // The first spelling seen is the one kept for display.
        assert_eq!(device.evidence[0].value, "Office Printer");
    }

    #[test]
    fn evidence_order_does_not_change_the_stored_result() {
        let a = Evidence::new(
            DiscoverySource::Mdns,
            EvidenceKind::Service,
            "_ipp._tcp",
            "_ipp._tcp",
            Confidence::High,
        );
        let b = Evidence::new(
            DiscoverySource::Ssdp,
            EvidenceKind::Manufacturer,
            "",
            "Acme",
            Confidence::High,
        );
        let ip: Ipv4Addr = "192.0.2.10".parse().unwrap();

        let mut forward = DiscoveredDevice::new(ip);
        forward.add(a.clone());
        forward.add(b.clone());
        forward.sort();

        let mut backward = DiscoveredDevice::new(ip);
        backward.add(b);
        backward.add(a);
        backward.sort();

        assert_eq!(forward, backward);
    }

    #[test]
    fn discovery_mode_demands_both_protocols_and_an_uninterrupted_pass() {
        let full = DiscoveryReport {
            mdns_attempted: true,
            ssdp_attempted: true,
            ..Default::default()
        };
        assert_eq!(full.mode(), DiscoveryMode::Full);

        let stopped = DiscoveryReport {
            interrupted: true,
            ..full.clone()
        };
        assert_eq!(stopped.mode(), DiscoveryMode::Partial);

        let one_protocol = DiscoveryReport {
            mdns_attempted: true,
            ssdp_attempted: false,
            ..Default::default()
        };
        assert_eq!(one_protocol.mode(), DiscoveryMode::Partial);

        assert_eq!(
            DiscoveryReport::skipped("Remote target").mode(),
            DiscoveryMode::None
        );
    }

    fn complete_report() -> DiscoveryReport {
        DiscoveryReport {
            mdns_attempted: true,
            ssdp_attempted: true,
            ..Default::default()
        }
    }

    #[test]
    fn quality_is_complete_only_when_nothing_was_cut_short() {
        let report = complete_report();
        assert_eq!(report.quality(), DiscoveryQuality::Complete);
        assert_eq!(report.quality_reason(), None);
    }

    #[test]
    fn a_stopped_scan_reads_as_interrupted_before_anything_else() {
        let report = DiscoveryReport {
            interrupted: true,
            // Also limited, but Stop is the thing that actually happened.
            mdns_socket_failed: true,
            ..complete_report()
        };
        assert_eq!(report.quality(), DiscoveryQuality::Interrupted);
        assert_eq!(report.quality_reason(), Some("Scan stopped"));
    }

    #[test]
    fn a_pass_that_never_started_reads_as_skipped() {
        let report = DiscoveryReport::skipped("Remote target");
        assert_eq!(report.quality(), DiscoveryQuality::Skipped);
        assert_eq!(report.quality_reason(), Some("Not run"));
    }

    #[test]
    fn every_observed_limitation_reads_as_limited_with_its_own_reason() {
        let cases: Vec<(DiscoveryReport, &str)> = vec![
            (
                DiscoveryReport {
                    mdns_socket_failed: true,
                    ..complete_report()
                },
                "mDNS socket unavailable",
            ),
            (
                DiscoveryReport {
                    ssdp_socket_failed: true,
                    ..complete_report()
                },
                "SSDP socket unavailable",
            ),
            (
                DiscoveryReport {
                    mdns_socket_failed: true,
                    ssdp_socket_failed: true,
                    ..complete_report()
                },
                "No discovery socket",
            ),
            (
                DiscoveryReport {
                    ssdp_attempted: false,
                    mdns_attempted: true,
                    ..Default::default()
                },
                "SSDP not run",
            ),
            (
                DiscoveryReport {
                    mdns_capped: true,
                    ..complete_report()
                },
                "Response limit reached",
            ),
            (
                DiscoveryReport {
                    descriptions_capped: true,
                    ..complete_report()
                },
                "Description limit reached",
            ),
            (
                DiscoveryReport {
                    descriptions_rejected: 2,
                    ..complete_report()
                },
                "Some descriptions refused",
            ),
        ];
        for (report, reason) in cases {
            assert_eq!(report.quality(), DiscoveryQuality::Limited, "{reason}");
            assert_eq!(report.quality_reason(), Some(reason));
        }
    }

    #[test]
    fn quality_round_trips_and_never_claims_a_firewall() {
        for quality in [
            DiscoveryQuality::Complete,
            DiscoveryQuality::Limited,
            DiscoveryQuality::Skipped,
            DiscoveryQuality::Interrupted,
        ] {
            assert_eq!(DiscoveryQuality::parse(quality.as_str()), quality);
            assert!(!quality.label().is_empty());
        }
        // ArcScan cannot observe a firewall, so it must never name one.
        let mut every_reason: Vec<&str> = Vec::new();
        for report in [
            complete_report(),
            DiscoveryReport {
                mdns_socket_failed: true,
                ssdp_socket_failed: true,
                ..complete_report()
            },
            DiscoveryReport {
                interrupted: true,
                ..complete_report()
            },
            DiscoveryReport::skipped("Remote target"),
        ] {
            every_reason.extend(report.quality_reason());
        }
        assert!(every_reason
            .iter()
            .all(|r| !r.to_lowercase().contains("firewall")));
    }

    #[test]
    fn a_type_override_value_is_refused_unless_it_is_one_of_the_words() {
        for kind in DeviceType::ALL {
            assert_eq!(DeviceType::parse_strict(kind.as_str()), Some(kind));
        }
        for bogus in ["toaster", "", "Printer", "printer ", "media device"] {
            assert_eq!(DeviceType::parse_strict(bogus), None, "{bogus:?}");
        }
        // The forgiving parser still exists for values read back out of the
        // database, where refusing would make a row unrenderable.
        assert_eq!(DeviceType::parse("toaster"), DeviceType::Unknown);
    }
}
