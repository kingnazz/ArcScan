//! Persistent device identity and change detection.
//!
//! A network inventory is only useful if the same physical device is recognised
//! across scans. IP addresses cannot do that: DHCP hands out a different lease
//! and the device looks brand new, which is exactly the false positive v1.6
//! produced. So identity is resolved in priority order:
//!
//! 1. **MAC address** — the only stable hardware identifier a read-only scan can
//!    see. Normalized so `a0:ce:c8:d:cf:d1`, `A0-CE-C8-0D-CF-D1` and
//!    `a0cec80dcfd1` all resolve to the same device. Highest confidence.
//! 2. **Hostname plus vendor** — for routed targets, where ARP gives us nothing.
//!    Both together, because `printer` on its own collides across sites. Medium
//!    confidence. When the vendor is absent the hostname stands alone, which is
//!    lower confidence still, so *generic* hostnames (`printer`, `router`,
//!    `localhost`, `unknown`, …) are refused as identities: an ambiguous name
//!    falls through to the IP rule and creates a separate device rather than
//!    silently merging two unrelated ones.
//! 3. **IP address** — last resort, and the only case where a DHCP change still
//!    reads as a new device. Nothing better exists for a host that answers with
//!    no MAC, no name and no vendor.
//!
//! Every identity is additionally scoped to a network scope (see
//! [`crate::db`]): matching never crosses scope boundaries, so the same key on
//! two different client networks is two different devices.
//!
//! Everything in this module is pure so the matching and diff rules can be
//! tested without a database or a network.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::ports;
use crate::scanner::HostResult;

/// Normalize a MAC address for identity comparison and storage.
///
/// Accepts colon- or dash-separated octets with or without zero padding, and
/// bare 12-digit hex. Returns `None` for the broadcast and all-zero addresses,
/// which identify nothing, and for anything that is not a MAC.
pub fn normalize_mac(mac: &str) -> Option<String> {
    let hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase();
    if hex.len() != 12 {
        return None;
    }
    let joined = hex
        .as_bytes()
        .chunks(2)
        .map(|pair| std::str::from_utf8(pair).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(":");
    if joined == "FF:FF:FF:FF:FF:FF" || joined == "00:00:00:00:00:00" {
        return None;
    }
    Some(joined)
}

/// How a device was matched, kept alongside the key so the UI and the tests can
/// explain why two observations were treated as the same device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentitySource {
    Mac,
    HostnameVendor,
    Ip,
}

/// The stable key a device is stored and matched under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub key: String,
    pub source: IdentitySource,
    /// Normalized MAC, when one was available.
    pub mac: Option<String>,
}

/// Hostnames too generic to identify a device on their own. Any of these
/// without a vendor to disambiguate falls through to IP identity, because two
/// unrelated devices called `printer` are far more likely than one printer.
const GENERIC_HOSTNAMES: &[&str] = &[
    "localhost",
    "localhost.localdomain",
    "printer",
    "router",
    "switch",
    "gateway",
    "server",
    "nas",
    "camera",
    "device",
    "host",
    "default",
    "unknown",
];

/// True when a hostname is too generic to stand alone as a device identity.
pub fn is_generic_hostname(hostname: &str) -> bool {
    let lower = hostname.trim().to_ascii_lowercase();
    GENERIC_HOSTNAMES.contains(&lower.as_str())
}

/// Resolve the identity of one observation, applying the confidence rules
/// documented at the top of this module.
pub fn identify(host: &HostResult) -> Identity {
    if let Some(mac) = host.mac.as_deref().and_then(normalize_mac) {
        return Identity {
            key: format!("mac:{mac}"),
            source: IdentitySource::Mac,
            mac: Some(mac),
        };
    }
    let hostname = host
        .hostname
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let vendor = host
        .vendor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(hostname) = hostname {
        // Without a vendor, a generic hostname is refused: it is not evidence
        // of identity, and merging on it would mix unrelated devices.
        let usable = vendor.is_some() || !is_generic_hostname(hostname);
        if usable {
            return Identity {
                key: format!(
                    "hv:{}|{}",
                    hostname.to_ascii_lowercase(),
                    vendor.unwrap_or("").to_ascii_lowercase()
                ),
                source: IdentitySource::HostnameVendor,
                mac: None,
            };
        }
    }
    Identity {
        key: format!("ip:{}", host.ip.trim()),
        source: IdentitySource::Ip,
        mac: None,
    }
}

/// How the operator has classified a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    /// Seen, never labelled.
    #[default]
    Unclassified,
    /// Recognised and expected on this network.
    Known,
    /// Recognised, and its services are deliberate.
    Trusted,
    /// Recognised, and the operator wants to be told when it changes.
    Watched,
    /// Recognised, and its changes are not worth reviewing. History is kept; new
    /// change events for the device are recorded already-ignored so they stay
    /// out of the default inbox without being lost.
    Ignored,
}

impl DeviceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceStatus::Unclassified => "unclassified",
            DeviceStatus::Known => "known",
            DeviceStatus::Trusted => "trusted",
            DeviceStatus::Watched => "watched",
            DeviceStatus::Ignored => "ignored",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "known" => DeviceStatus::Known,
            "trusted" => DeviceStatus::Trusted,
            "watched" => DeviceStatus::Watched,
            "ignored" => DeviceStatus::Ignored,
            _ => DeviceStatus::Unclassified,
        }
    }
}

/// Whether a device answered the most recent *completed* scan that could have
/// seen it.
///
/// ArcScan does not watch a network continuously, so these three values are the
/// only honest ones. The rules are applied in [`crate::db::Db::inventory`] and
/// are, in full:
///
/// * A network scope's **reference scan** is its most recent scan that both
///   completed (was not stopped early) and carries a real coverage key. A scan
///   recorded before coverage keys existed cannot say which ports it checked, so
///   it is never a reference.
/// * **Present** — the device was observed by that reference scan.
/// * **Missing** — the device was not observed by the reference scan, but it was
///   observed by at least one earlier completed scan with the *same* target and
///   coverage, so the reference scan genuinely looked where the device used to
///   be and did not find it.
/// * **Unknown** — everything else: the scope has no reference scan (only
///   partial scans, only legacy scans, or none at all), or the device has only
///   ever been seen under a different target or coverage, so its absence proves
///   nothing.
///
/// A partial scan can therefore never make a device Missing: it is excluded from
/// being a reference and from the compatible-history test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PresenceState {
    /// Observed in the latest completed scan that covers this device's network.
    Present,
    /// Observed before under the same coverage, absent from the latest one.
    Missing,
    /// Presence cannot be determined from completed scans.
    #[default]
    Unknown,
}

/// What a persistent change event records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    /// Never observed before this scan.
    DeviceAdded,
    /// Known to the inventory, absent from the baseline scan, back now.
    DeviceReturned,
    /// In the baseline scan, absent from this one.
    DeviceMissing,
    IpChanged,
    HostnameChanged,
    VendorChanged,
    OsChanged,
    MacChanged,
    PortsChanged,
    // --- Discovery-derived changes (v1.8.2) ---------------------------------
    //
    // Recorded only when both the scan and its baseline ran a *full* discovery
    // pass, and only for facts stable enough to be worth a person's attention.
    // The rules that keep these quiet live in `crate::db::record_discovery_events`.
    /// The name a device advertises for itself changed, at high confidence.
    DetectedNameChanged,
    /// The kind of device ArcScan is confident it is changed.
    DeviceTypeChanged,
    /// A meaningful advertised service appeared.
    ServiceAppeared,
    /// A meaningful advertised service stopped being advertised, and stayed
    /// gone long enough to be believed.
    ServiceDisappeared,
    /// The manufacturer or model a device reports changed.
    ModelChanged,
}

impl ChangeType {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeType::DeviceAdded => "device_added",
            ChangeType::DeviceReturned => "device_returned",
            ChangeType::DeviceMissing => "device_missing",
            ChangeType::IpChanged => "ip_changed",
            ChangeType::HostnameChanged => "hostname_changed",
            ChangeType::VendorChanged => "vendor_changed",
            ChangeType::OsChanged => "os_changed",
            ChangeType::MacChanged => "mac_changed",
            ChangeType::PortsChanged => "ports_changed",
            ChangeType::DetectedNameChanged => "detected_name_changed",
            ChangeType::DeviceTypeChanged => "device_type_changed",
            ChangeType::ServiceAppeared => "service_appeared",
            ChangeType::ServiceDisappeared => "service_disappeared",
            ChangeType::ModelChanged => "model_changed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "device_added" => ChangeType::DeviceAdded,
            "device_returned" => ChangeType::DeviceReturned,
            "device_missing" => ChangeType::DeviceMissing,
            "ip_changed" => ChangeType::IpChanged,
            "hostname_changed" => ChangeType::HostnameChanged,
            "vendor_changed" => ChangeType::VendorChanged,
            "os_changed" => ChangeType::OsChanged,
            "mac_changed" => ChangeType::MacChanged,
            "ports_changed" => ChangeType::PortsChanged,
            "detected_name_changed" => ChangeType::DetectedNameChanged,
            "device_type_changed" => ChangeType::DeviceTypeChanged,
            "service_appeared" => ChangeType::ServiceAppeared,
            "service_disappeared" => ChangeType::ServiceDisappeared,
            "model_changed" => ChangeType::ModelChanged,
            _ => return None,
        })
    }

    /// The change type a [`FieldChange`] becomes, or `None` for fields that are
    /// not worth an inbox entry on their own.
    pub fn for_field(field: &str) -> Option<Self> {
        Some(match field {
            "ip" => ChangeType::IpChanged,
            "hostname" => ChangeType::HostnameChanged,
            "vendor" => ChangeType::VendorChanged,
            "os_guess" => ChangeType::OsChanged,
            "mac" => ChangeType::MacChanged,
            "ports" => ChangeType::PortsChanged,
            _ => return None,
        })
    }
}

/// Where a change event sits in the review inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChangeState {
    #[default]
    Unreviewed,
    Acknowledged,
    /// Deliberately not worth reviewing. The event is kept and can be filtered
    /// back into view; it is simply out of the default inbox.
    Ignored,
}

impl ChangeState {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeState::Unreviewed => "unreviewed",
            ChangeState::Acknowledged => "acknowledged",
            ChangeState::Ignored => "ignored",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "unreviewed" => ChangeState::Unreviewed,
            "acknowledged" => ChangeState::Acknowledged,
            "ignored" => ChangeState::Ignored,
            _ => return None,
        })
    }
}

/// A device in the persistent inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: i64,
    /// The network scope this device belongs to; identity never crosses it.
    #[serde(default)]
    pub network_scope_id: Option<i64>,
    pub identity_key: String,
    pub identity_source: IdentitySource,
    pub mac: Option<String>,
    /// Operator-supplied friendly name, which always wins for display.
    pub custom_name: Option<String>,
    pub hostname: Option<String>,
    pub vendor: Option<String>,
    pub last_ip: Option<String>,
    pub first_seen: String,
    pub last_seen: String,
    pub status: DeviceStatus,
    pub notes: Option<String>,
    /// How many scans have observed this device.
    #[serde(default)]
    pub observation_count: i64,
    /// The operator's device-type correction, or `None` for Auto.
    ///
    /// A correction to what ArcScan *calls* the device, deliberately alongside
    /// the name, the status and the notes: it is an operator label, and like
    /// every other operator label it has no effect on `identity_key`,
    /// `identity_source`, `mac`, the network scope or presence. Nothing in
    /// this module's matching or comparison rules reads it.
    #[serde(default)]
    pub user_device_type: Option<String>,
}

/// The name to show for a device, in the order a person would expect.
pub fn display_name(
    custom_name: Option<&str>,
    hostname: Option<&str>,
    vendor: Option<&str>,
    ip: &str,
) -> String {
    display_name_detected(custom_name, None, hostname, vendor, ip)
}

/// The same order, with a name the device advertised for itself slotted in.
///
/// A detected name sits above the reverse-DNS hostname because it is what the
/// device's owner typed into it, while a hostname is usually what DHCP made up.
/// It sits below `custom_name` for the reason that governs this whole release:
/// a name a person chose is never replaced by one a device announced.
///
/// Only a *strong* detected name should be passed here — the full ranking,
/// including how generic names are demoted, lives in
/// [`crate::discovery::names::resolve`], which is what decides whether a device
/// has a detected name at all.
pub fn display_name_detected(
    custom_name: Option<&str>,
    detected_name: Option<&str>,
    hostname: Option<&str>,
    vendor: Option<&str>,
    ip: &str,
) -> String {
    fn pick(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|v| !v.is_empty())
    }
    if let Some(name) = pick(custom_name) {
        return name.to_string();
    }
    if let Some(name) = pick(detected_name) {
        return name.to_string();
    }
    if let Some(name) = pick(hostname) {
        return name.to_string();
    }
    if let Some(vendor) = pick(vendor) {
        // "Apple device (192.168.1.20)" reads better than a bare vendor string
        // repeated across every unnamed device.
        return format!("{vendor} ({ip})");
    }
    ip.to_string()
}

/// One field that differs between two observations of the same device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldChange {
    /// Machine name, e.g. `ip`.
    pub field: String,
    /// Human label, e.g. `IP address`.
    pub label: String,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Ports gained, for the `ports` field.
    #[serde(default)]
    pub added_ports: Vec<u16>,
    /// Ports lost, for the `ports` field.
    #[serde(default)]
    pub removed_ports: Vec<u16>,
}

/// What happened to one device between two scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeKind {
    /// Never seen by any previous scan of this target.
    New,
    /// Seen before, absent from the baseline scan, back now.
    Returned,
    /// In the baseline scan, not in this one.
    Missing,
    /// Present in both, with at least one field different.
    Changed,
}

/// A device entry in a scan comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceDiff {
    pub kind: ChangeKind,
    pub device_id: Option<i64>,
    pub name: String,
    pub ip: String,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub hostname: Option<String>,
    /// When the device was last observed, for missing devices.
    pub last_seen: Option<String>,
    pub fields: Vec<FieldChange>,
}

/// The full comparison between a scan and its baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanComparison {
    pub scan_id: i64,
    pub baseline_scan_id: Option<i64>,
    pub baseline_created_at: Option<String>,
    pub baseline_target: Option<String>,
    /// Set when no compatible earlier scan exists, so the UI can say why there
    /// is nothing to compare instead of implying nothing changed.
    pub reason: Option<String>,
    pub added: Vec<DeviceDiff>,
    pub removed: Vec<DeviceDiff>,
    pub changed: Vec<DeviceDiff>,
}

impl ScanComparison {
    pub fn empty(scan_id: i64, reason: impl Into<String>) -> Self {
        ScanComparison {
            scan_id,
            baseline_scan_id: None,
            baseline_created_at: None,
            baseline_target: None,
            reason: Some(reason.into()),
            added: Vec::new(),
            removed: Vec::new(),
            changed: Vec::new(),
        }
    }
}

/// One observation paired with the device it resolved to.
#[derive(Debug, Clone)]
pub struct IdentifiedHost {
    pub device_id: Option<i64>,
    pub identity_key: String,
    pub custom_name: Option<String>,
    /// True when the device existed in the inventory before the baseline scan.
    pub previously_known: bool,
    pub host: HostResult,
}

impl IdentifiedHost {
    pub fn from_host(host: HostResult) -> Self {
        let identity = identify(&host);
        IdentifiedHost {
            device_id: None,
            identity_key: identity.key,
            custom_name: None,
            previously_known: false,
            host,
        }
    }

    fn name(&self) -> String {
        display_name(
            self.custom_name.as_deref(),
            self.host.hostname.as_deref(),
            self.host.vendor.as_deref(),
            &self.host.ip,
        )
    }
}

/// Compare two identified host sets and produce the added, removed and changed
/// lists. Matching is by identity key, so a device whose IP changed is reported
/// as changed rather than as one new plus one missing device.
pub fn compare(
    scan_id: i64,
    baseline: &[IdentifiedHost],
    current: &[IdentifiedHost],
) -> ScanComparison {
    let baseline_by_key: HashMap<&str, &IdentifiedHost> = baseline
        .iter()
        .map(|h| (h.identity_key.as_str(), h))
        .collect();
    let current_by_key: HashMap<&str, &IdentifiedHost> = current
        .iter()
        .map(|h| (h.identity_key.as_str(), h))
        .collect();

    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut removed = Vec::new();

    for entry in current {
        match baseline_by_key.get(entry.identity_key.as_str()) {
            Some(before) => {
                let fields = diff_fields(&before.host, &entry.host);
                if !fields.is_empty() {
                    changed.push(DeviceDiff {
                        kind: ChangeKind::Changed,
                        device_id: entry.device_id,
                        name: entry.name(),
                        ip: entry.host.ip.clone(),
                        mac: entry.host.mac.clone(),
                        vendor: entry.host.vendor.clone(),
                        hostname: entry.host.hostname.clone(),
                        last_seen: Some(entry.host.last_seen.clone()),
                        fields,
                    });
                }
            }
            None => added.push(DeviceDiff {
                // A device the inventory has seen before, just not in the
                // baseline scan, is a return rather than an arrival.
                kind: if entry.previously_known {
                    ChangeKind::Returned
                } else {
                    ChangeKind::New
                },
                device_id: entry.device_id,
                name: entry.name(),
                ip: entry.host.ip.clone(),
                mac: entry.host.mac.clone(),
                vendor: entry.host.vendor.clone(),
                hostname: entry.host.hostname.clone(),
                last_seen: Some(entry.host.last_seen.clone()),
                fields: Vec::new(),
            }),
        }
    }

    for entry in baseline {
        if !current_by_key.contains_key(entry.identity_key.as_str()) {
            removed.push(DeviceDiff {
                kind: ChangeKind::Missing,
                device_id: entry.device_id,
                name: entry.name(),
                ip: entry.host.ip.clone(),
                mac: entry.host.mac.clone(),
                vendor: entry.host.vendor.clone(),
                hostname: entry.host.hostname.clone(),
                last_seen: Some(entry.host.last_seen.clone()),
                fields: Vec::new(),
            });
        }
    }

    let by_ip = |a: &DeviceDiff, b: &DeviceDiff| ip_order(&a.ip).cmp(&ip_order(&b.ip));
    added.sort_by(by_ip);
    changed.sort_by(by_ip);
    removed.sort_by(by_ip);

    ScanComparison {
        scan_id,
        baseline_scan_id: None,
        baseline_created_at: None,
        baseline_target: None,
        reason: None,
        added,
        removed,
        changed,
    }
}

fn ip_order(ip: &str) -> u32 {
    ip.parse::<std::net::Ipv4Addr>().map(u32::from).unwrap_or(0)
}

/// Field-level differences between two observations of the same device.
///
/// Only meaningful changes are reported. Response times and timestamps differ on
/// every scan by nature, so they are never treated as changes: doing so would
/// mark the whole network as changed every time.
pub fn diff_fields(before: &HostResult, after: &HostResult) -> Vec<FieldChange> {
    let mut out = Vec::new();

    if before.ip != after.ip {
        out.push(FieldChange {
            field: "ip".into(),
            label: "IP address".into(),
            from: Some(before.ip.clone()),
            to: Some(after.ip.clone()),
            added_ports: Vec::new(),
            removed_ports: Vec::new(),
        });
    }
    for (field, label, from, to) in [
        ("hostname", "Hostname", &before.hostname, &after.hostname),
        ("vendor", "Manufacturer", &before.vendor, &after.vendor),
        (
            "os_guess",
            "Operating system",
            &before.os_guess,
            &after.os_guess,
        ),
        ("mac", "MAC address", &before.mac, &after.mac),
    ] {
        let a = from.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let b = to.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if a != b {
            out.push(FieldChange {
                field: field.into(),
                label: label.into(),
                from: a.map(str::to_string),
                to: b.map(str::to_string),
                added_ports: Vec::new(),
                removed_ports: Vec::new(),
            });
        }
    }

    let before_ports: BTreeSet<u16> = before.open_ports.iter().copied().collect();
    let after_ports: BTreeSet<u16> = after.open_ports.iter().copied().collect();
    let opened: Vec<u16> = after_ports.difference(&before_ports).copied().collect();
    let closed: Vec<u16> = before_ports.difference(&after_ports).copied().collect();
    if !opened.is_empty() || !closed.is_empty() {
        out.push(FieldChange {
            field: "ports".into(),
            label: "Open services".into(),
            from: Some(format_ports(&before.open_ports)),
            to: Some(format_ports(&after.open_ports)),
            added_ports: opened,
            removed_ports: closed,
        });
    }
    out
}

/// Render a port list the way the UI shows it: service name and number.
pub fn format_ports(list: &[u16]) -> String {
    if list.is_empty() {
        return "none".into();
    }
    list.iter()
        .map(|p| match ports::service_name(*p) {
            Some(name) => format!("{name} · {p}"),
            None => p.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(ip: &str, mac: Option<&str>, hostname: Option<&str>, ports: &[u16]) -> HostResult {
        HostResult {
            ip: ip.into(),
            hostname: hostname.map(str::to_string),
            mac: mac.map(str::to_string),
            vendor: mac.map(|_| "Acme Networks".to_string()),
            open_ports: ports.to_vec(),
            response_ms: Some(3),
            icmp_ms: Some(2.5),
            tcp_ms: Some(3.1),
            ttl: Some(64),
            os_guess: Some("Linux/Unix/macOS".into()),
            discovery: None,
            last_seen: "2026-07-01T10:00:00+00:00".into(),
        }
    }

    fn identified(host: HostResult) -> IdentifiedHost {
        IdentifiedHost::from_host(host)
    }

    #[test]
    fn normalizes_every_common_mac_form() {
        let expected = Some("A0:CE:C8:0D:CF:D1".to_string());
        assert_eq!(normalize_mac("a0:ce:c8:0d:cf:d1"), expected);
        assert_eq!(normalize_mac("A0-CE-C8-0D-CF-D1"), expected);
        assert_eq!(normalize_mac("a0cec80dcfd1"), expected);
        assert_eq!(normalize_mac("A0:CE:C8:0D:CF:D1"), expected);
    }

    #[test]
    fn rejects_meaningless_macs() {
        assert_eq!(normalize_mac("FF:FF:FF:FF:FF:FF"), None);
        assert_eq!(normalize_mac("00:00:00:00:00:00"), None);
        assert_eq!(normalize_mac("nope"), None);
        assert_eq!(normalize_mac("a0:ce:c8"), None);
    }

    #[test]
    fn mac_is_the_primary_identity() {
        let id = identify(&host(
            "10.0.0.5",
            Some("aa:bb:cc:dd:ee:01"),
            Some("nas"),
            &[],
        ));
        assert_eq!(id.source, IdentitySource::Mac);
        assert_eq!(id.key, "mac:AA:BB:CC:DD:EE:01");
    }

    #[test]
    fn hostname_and_vendor_identify_a_host_without_a_mac() {
        let mut h = host("10.0.0.5", None, Some("Printer-01"), &[]);
        h.vendor = Some("HP Inc.".into());
        let id = identify(&h);
        assert_eq!(id.source, IdentitySource::HostnameVendor);
        assert_eq!(id.key, "hv:printer-01|hp inc.");
    }

    #[test]
    fn ip_is_the_last_resort_identity() {
        let mut h = host("10.0.0.5", None, None, &[]);
        h.vendor = None;
        let id = identify(&h);
        assert_eq!(id.source, IdentitySource::Ip);
        assert_eq!(id.key, "ip:10.0.0.5");
    }

    #[test]
    fn dhcp_address_change_is_not_a_new_device() {
        // The v1.6 false positive: the same printer on a new lease.
        let before = vec![identified(host(
            "192.168.1.42",
            Some("aa:bb:cc:dd:ee:02"),
            Some("printer-old"),
            &[80],
        ))];
        let after = vec![identified(host(
            "192.168.1.57",
            Some("aa:bb:cc:dd:ee:02"),
            Some("front-office-printer"),
            &[80, 443],
        ))];

        let diff = compare(2, &before, &after);
        assert!(diff.added.is_empty(), "{:?}", diff.added);
        assert!(diff.removed.is_empty(), "{:?}", diff.removed);
        assert_eq!(diff.changed.len(), 1);

        let fields = &diff.changed[0].fields;
        let ip = fields.iter().find(|f| f.field == "ip").unwrap();
        assert_eq!(ip.from.as_deref(), Some("192.168.1.42"));
        assert_eq!(ip.to.as_deref(), Some("192.168.1.57"));

        let name = fields.iter().find(|f| f.field == "hostname").unwrap();
        assert_eq!(name.to.as_deref(), Some("front-office-printer"));

        let svc = fields.iter().find(|f| f.field == "ports").unwrap();
        assert_eq!(svc.added_ports, vec![443]);
        assert!(svc.removed_ports.is_empty());
    }

    #[test]
    fn detects_new_and_missing_devices() {
        let before = vec![identified(host(
            "10.0.0.2",
            Some("aa:bb:cc:00:00:02"),
            Some("laptop"),
            &[],
        ))];
        let after = vec![identified(host(
            "10.0.0.3",
            Some("aa:bb:cc:00:00:03"),
            Some("tablet"),
            &[],
        ))];
        let diff = compare(2, &before, &after);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].kind, ChangeKind::New);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].kind, ChangeKind::Missing);
        assert!(diff.changed.is_empty());
    }

    #[test]
    fn a_previously_known_device_is_a_return_not_an_arrival() {
        let mut entry = identified(host(
            "10.0.0.9",
            Some("aa:bb:cc:00:00:09"),
            Some("phone"),
            &[],
        ));
        entry.previously_known = true;
        let diff = compare(2, &[], &[entry]);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].kind, ChangeKind::Returned);
    }

    #[test]
    fn detects_closed_ports() {
        let before = vec![identified(host(
            "10.0.0.4",
            Some("aa:bb:cc:00:00:04"),
            Some("srv"),
            &[22, 80, 443],
        ))];
        let after = vec![identified(host(
            "10.0.0.4",
            Some("aa:bb:cc:00:00:04"),
            Some("srv"),
            &[22, 443],
        ))];
        let diff = compare(2, &before, &after);
        let svc = diff.changed[0]
            .fields
            .iter()
            .find(|f| f.field == "ports")
            .unwrap();
        assert_eq!(svc.removed_ports, vec![80]);
        assert!(svc.added_ports.is_empty());
    }

    #[test]
    fn latency_and_timestamps_are_never_changes() {
        let mut before = host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("srv"), &[22]);
        let mut after = before.clone();
        before.response_ms = Some(2);
        before.icmp_ms = Some(1.5);
        after.response_ms = Some(180);
        after.icmp_ms = Some(179.2);
        after.tcp_ms = Some(200.0);
        after.last_seen = "2026-07-02T11:00:00+00:00".into();
        assert!(diff_fields(&before, &after).is_empty());
    }

    #[test]
    fn identical_scans_produce_no_changes() {
        let hosts = vec![
            identified(host(
                "10.0.0.2",
                Some("aa:bb:cc:00:00:02"),
                Some("a"),
                &[80],
            )),
            identified(host(
                "10.0.0.3",
                Some("aa:bb:cc:00:00:03"),
                Some("b"),
                &[22],
            )),
        ];
        let diff = compare(2, &hosts, &hosts);
        assert_eq!(
            diff.added.len() + diff.removed.len() + diff.changed.len(),
            0
        );
    }

    #[test]
    fn diffs_are_ordered_by_address() {
        let after = vec![
            identified(host("10.0.0.30", Some("aa:bb:cc:00:00:30"), None, &[])),
            identified(host("10.0.0.4", Some("aa:bb:cc:00:00:04"), None, &[])),
        ];
        let diff = compare(1, &[], &after);
        assert_eq!(diff.added[0].ip, "10.0.0.4");
        assert_eq!(diff.added[1].ip, "10.0.0.30");
    }

    #[test]
    fn display_name_prefers_the_operator_label() {
        assert_eq!(
            display_name(
                Some("Front Office Printer"),
                Some("printer-01"),
                None,
                "10.0.0.5"
            ),
            "Front Office Printer"
        );
        assert_eq!(
            display_name(None, Some("printer-01"), Some("HP"), "10.0.0.5"),
            "printer-01"
        );
        assert_eq!(
            display_name(None, None, Some("HP Inc."), "10.0.0.5"),
            "HP Inc. (10.0.0.5)"
        );
        assert_eq!(display_name(None, None, None, "10.0.0.5"), "10.0.0.5");
        // Blank strings are treated as absent rather than shown as an empty name.
        assert_eq!(
            display_name(Some("  "), Some(""), None, "10.0.0.5"),
            "10.0.0.5"
        );
    }

    #[test]
    fn formats_ports_with_service_names() {
        assert_eq!(format_ports(&[]), "none");
        assert_eq!(format_ports(&[443, 3389]), "HTTPS · 443, RDP · 3389");
        assert_eq!(format_ports(&[64999]), "64999");
    }

    #[test]
    fn device_status_round_trips() {
        for status in [
            DeviceStatus::Unclassified,
            DeviceStatus::Known,
            DeviceStatus::Trusted,
            DeviceStatus::Watched,
            DeviceStatus::Ignored,
        ] {
            assert_eq!(DeviceStatus::parse(status.as_str()), status);
        }
        assert_eq!(DeviceStatus::parse("nonsense"), DeviceStatus::Unclassified);
    }

    #[test]
    fn presence_state_crosses_the_boundary_as_the_ui_expects() {
        // The frontend switches on these three strings; renaming one silently
        // would make every device read as neither present nor missing.
        assert_eq!(
            serde_json::to_string(&PresenceState::Present).unwrap(),
            "\"present\""
        );
        assert_eq!(
            serde_json::to_string(&PresenceState::Missing).unwrap(),
            "\"missing\""
        );
        assert_eq!(
            serde_json::to_string(&PresenceState::Unknown).unwrap(),
            "\"unknown\""
        );
        // Presence is never assumed: the default is Unknown, not Present.
        assert_eq!(PresenceState::default(), PresenceState::Unknown);
    }

    #[test]
    fn change_type_round_trips_and_rejects_unknown_values() {
        for kind in [
            ChangeType::DeviceAdded,
            ChangeType::DeviceReturned,
            ChangeType::DeviceMissing,
            ChangeType::IpChanged,
            ChangeType::HostnameChanged,
            ChangeType::VendorChanged,
            ChangeType::OsChanged,
            ChangeType::MacChanged,
            ChangeType::PortsChanged,
            ChangeType::DetectedNameChanged,
            ChangeType::DeviceTypeChanged,
            ChangeType::ServiceAppeared,
            ChangeType::ServiceDisappeared,
            ChangeType::ModelChanged,
        ] {
            assert_eq!(ChangeType::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ChangeType::parse("ticket_raised"), None);
    }

    #[test]
    fn every_reported_field_change_maps_to_a_change_type() {
        // diff_fields is the only producer of change events for a device that
        // was present in both scans, so every field it can emit must have a
        // type. A field with no mapping would silently vanish from the inbox.
        let mut before = host("10.0.0.4", Some("aa:bb:cc:00:00:04"), Some("srv"), &[22]);
        before.os_guess = Some("Windows".into());
        let mut after = host("10.0.0.5", Some("aa:bb:cc:00:00:05"), Some("srv2"), &[443]);
        after.vendor = Some("Other Vendor".into());
        after.os_guess = Some("Linux/Unix/macOS".into());

        let fields = diff_fields(&before, &after);
        assert_eq!(fields.len(), 6, "{fields:?}");
        for field in &fields {
            assert!(
                ChangeType::for_field(&field.field).is_some(),
                "no change type for {}",
                field.field
            );
        }
    }

    #[test]
    fn change_state_round_trips() {
        for state in [
            ChangeState::Unreviewed,
            ChangeState::Acknowledged,
            ChangeState::Ignored,
        ] {
            assert_eq!(ChangeState::parse(state.as_str()), Some(state));
        }
        assert_eq!(ChangeState::parse("closed"), None);
    }
}
