//! Structured topology diagnostics.
//!
//! Built from the tables ArcScan already collected and from the same
//! correlation decisions that produce links. Nothing here is parsed out of
//! log lines, and nothing here is allowed to change confidence or matching.
//! Credentials are not a field on any of these types.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::collect::{
    is_unicast_mac, normalize_mac, CdpNeighbor, DeviceView, LldpNeighbor, ProbeState, TableProbe,
};
use crate::error::redact_secrets;
use crate::model::{TopologyConfidence, TopologyDeviceFailure, TopologySnapshot, TopologyTarget};

const MAX_PORT_DETAILS: usize = 12;
const MAX_LISTED_SUPPRESSIONS: usize = 16;
const MAX_LISTED_RELATIONSHIPS: usize = 24;

const EXPORT_WARNING: &str = "This file contains network inventory information (IP addresses, MAC addresses, and device names). It does not contain SNMP credentials. ArcScan does not upload it.";

/// How one SNMP table walk finished. `Partial` is only used when several
/// walks inside one MIB disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MibState {
    Available,
    NoRows,
    TimedOut,
    WalkFailed,
    NotQueried,
    Partial,
}

impl MibState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::NoRows => "noRows",
            Self::TimedOut => "timedOut",
            Self::WalkFailed => "walkFailed",
            Self::NotQueried => "notQueried",
            Self::Partial => "partial",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnmpStatus {
    Responded,
    Timeout,
    AuthFailed,
    Unreachable,
    InvalidAddress,
    ProtocolError,
}

/// One MIB family, aggregated from the walks actually attempted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MibCoverage {
    pub mib: String,
    pub state: MibState,
    pub rows: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceStats {
    pub count: usize,
    pub up: usize,
    pub with_name: usize,
    pub with_speed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborObservation {
    pub local_port: String,
    pub remote_port: Option<String>,
    pub chassis_id: Option<String>,
    pub management_address: Option<String>,
    pub sys_name: Option<String>,
    pub resolved_device_id: Option<i64>,
    pub resolution: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborProtoDiag {
    pub state: MibState,
    pub neighbour_count: usize,
    pub resolved: usize,
    pub unresolved: usize,
    pub neighbours: Vec<NeighborObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FdbPortDiag {
    pub port_label: String,
    pub raw_port: u32,
    pub relevant_macs: usize,
    pub matched_inventory: usize,
    pub outcome: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FdbDiag {
    pub state: MibState,
    pub total_rows: usize,
    pub unicast_macs: usize,
    pub matched_inventory: usize,
    pub unresolved_macs: usize,
    pub single_mac_ports: usize,
    pub multi_mac_ports: usize,
    pub strong_links: usize,
    pub uplink_suppressions: usize,
    pub ports_omitted: usize,
    pub ports: Vec<FdbPortDiag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArpDiag {
    pub state: MibState,
    pub entries: usize,
    pub fdb_corroborations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VlanDiag {
    pub state: MibState,
    pub pvid_ports: usize,
    pub access_ports: usize,
    pub trunk_ports: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoeDiag {
    pub detection_state: MibState,
    pub wattage_state: MibState,
    pub enabled_ports: usize,
    pub ports_with_watts: usize,
    pub enabled_without_watts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMappingDiag {
    pub role: String,
    pub raw_port: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_port: Option<u32>,
    pub resolved_if_index: u32,
    pub display_label: String,
    pub resolution_source: String,
    pub fell_back_to_numeric: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipDiag {
    pub protocol: String,
    pub confidence: String,
    pub from_device_id: Option<i64>,
    pub to_device_id: Option<i64>,
    pub to_unresolved_id: Option<String>,
    pub from_port: Option<String>,
    pub to_port: Option<String>,
    pub resolution: String,
    pub why: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<PortMappingDiag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suppression {
    pub target_ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_device_id: Option<i64>,
    pub reason: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrelationNote {
    pub target_ip: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTopologyDiagnostics {
    pub target_ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_device_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub snmp_status: SnmpStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sys_name: Option<String>,
    pub mib_coverage: Vec<MibCoverage>,
    pub interfaces: InterfaceStats,
    pub lldp: NeighborProtoDiag,
    pub cdp: NeighborProtoDiag,
    pub fdb: FdbDiag,
    pub arp: ArpDiag,
    pub vlan: VlanDiag,
    pub poe: PoeDiag,
    pub relationships: Vec<RelationshipDiag>,
    /// Total relationships before the rendered list is capped.
    #[serde(default)]
    pub relationship_count: usize,
    pub relationships_omitted: usize,
    pub unresolved_peers: usize,
    pub suppressions: Vec<Suppression>,
    pub suppressions_omitted: usize,
    pub port_mappings: Vec<PortMappingDiag>,
    pub notes: Vec<String>,
    pub hints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zero_link_explanation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TopologyRunSummary {
    pub devices_queried: usize,
    pub devices_responding: usize,
    pub devices_failed: usize,
    pub lldp_cdp_neighbours: usize,
    pub fdb_relationships: usize,
    pub confirmed_links: usize,
    pub strong_links: usize,
    pub inferred_links: usize,
    pub unresolved_neighbours: usize,
    pub suppressed_candidates: usize,
    pub partial_snmp_devices: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyDiagnostics {
    pub kind: String,
    pub devices: Vec<DeviceTopologyDiagnostics>,
    pub run_summary: TopologyRunSummary,
    pub suppressions: Vec<Suppression>,
    pub correlation_notes: Vec<CorrelationNote>,
}

impl Default for TopologyDiagnostics {
    fn default() -> Self {
        Self {
            kind: "arcscan-topology-diagnostics".into(),
            devices: Vec::new(),
            run_summary: TopologyRunSummary::default(),
            suppressions: Vec::new(),
            correlation_notes: Vec::new(),
        }
    }
}

/// Facts recorded at the moment a link is accepted. Wording is generated
/// later from these fields so the sentence cannot drift from the decision.
#[derive(Debug, Clone)]
pub struct ObservedLink {
    pub source_ip: String,
    pub from_device_id: Option<i64>,
    pub to_device_id: Option<i64>,
    pub to_unresolved_id: Option<String>,
    pub protocol: String,
    pub confidence: String,
    pub from_port: Option<String>,
    pub to_port: Option<String>,
    pub raw_port: u32,
    pub port_role: String,
    pub chassis_or_mac: Option<String>,
    pub sys_name: Option<String>,
    pub management_address: Option<String>,
    pub resolution: String,
    pub arp_ip: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CorrelationTrace {
    pub links: Vec<ObservedLink>,
    pub suppressions: Vec<Suppression>,
    pub notes: Vec<CorrelationNote>,
}

impl CorrelationTrace {
    pub fn link(&mut self, link: ObservedLink) {
        self.links.push(link);
    }

    pub fn suppress(&mut self, suppression: Suppression) {
        self.suppressions.push(suppression);
    }

    pub fn note(&mut self, note: CorrelationNote) {
        self.notes.push(note);
    }
}

pub fn explain_port(view: &DeviceView, raw_port: u32, role: &str) -> PortMappingDiag {
    let direct = view.interfaces.contains_key(&raw_port);
    let bridge = view.bridge_port_if.get(&raw_port).copied();
    let lldp_name = view.lldp_local_ports.get(&raw_port);
    let resolved = view.resolve_if_index(raw_port);
    let iface = view.iface(resolved);
    let printable = iface.and_then(|candidate| {
        crate::display::first_printable([
            candidate.name.as_deref(),
            candidate.alias.as_deref(),
            candidate.descr.as_deref(),
        ])
    });
    let fell_back = printable.is_none();
    let display_label = printable.unwrap_or_else(|| resolved.to_string());
    let resolution_source = if direct {
        format!("ifIndex {raw_port} matched IF-MIB directly")
    } else if let Some(idx) = bridge {
        format!("bridge port {raw_port} → ifIndex {idx} via BRIDGE-MIB dot1dBasePortIfIndex")
    } else if lldp_name.is_some() && resolved != raw_port {
        format!("LLDP local-port table → IF-MIB ifIndex {resolved}")
    } else if fell_back {
        format!("numeric port {raw_port}; no IF-MIB name resolved")
    } else {
        format!("ifIndex {resolved}")
    };
    PortMappingDiag {
        role: role.to_string(),
        raw_port,
        bridge_port: bridge.map(|_| raw_port),
        resolved_if_index: resolved,
        display_label,
        resolution_source,
        fell_back_to_numeric: fell_back,
    }
}

pub fn inventory_label(targets: &[TopologyTarget], id: i64) -> String {
    targets
        .iter()
        .find(|target| target.device_id == Some(id))
        .and_then(|target| {
            target
                .detected_name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .or_else(|| target.hostname.clone())
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("device {id}"))
}

/// Hostname / sysName resemblance for a diagnostic sentence. Never an identity.
pub fn hostname_resemblance(targets: &[TopologyTarget], name: &str) -> Option<String> {
    let needle = name.trim();
    if needle.is_empty() {
        return None;
    }
    let mut matched: Option<String> = None;
    for target in targets {
        for candidate in [target.hostname.as_deref(), target.detected_name.as_deref()]
            .into_iter()
            .flatten()
        {
            if candidate.eq_ignore_ascii_case(needle) {
                if matched
                    .as_deref()
                    .is_some_and(|prev| !prev.eq_ignore_ascii_case(candidate))
                {
                    return Some(needle.to_string());
                }
                matched = Some(candidate.to_string());
            }
        }
    }
    matched
}

pub fn assemble(
    views: &[DeviceView],
    targets: &[TopologyTarget],
    snapshot: &TopologySnapshot,
    trace: &CorrelationTrace,
) -> TopologyDiagnostics {
    let mut devices: Vec<DeviceTopologyDiagnostics> = views
        .iter()
        .map(|view| device_from_view(view, targets, trace))
        .collect();
    devices.sort_by(|a, b| a.target_ip.cmp(&b.target_ip));

    let mut suppressions = trace.suppressions.clone();
    suppressions.sort_by(|a, b| {
        (&a.target_ip, &a.reason, &a.port_label).cmp(&(&b.target_ip, &b.reason, &b.port_label))
    });
    let mut notes = trace.notes.clone();
    notes.sort_by(|a, b| (&a.target_ip, &a.summary).cmp(&(&b.target_ip, &b.summary)));

    for view in views {
        let id = view.inventory_hint;
        for extra in malformed_suppressions(view, id) {
            if !suppressions
                .iter()
                .any(|item| item.reason == extra.reason && item.summary == extra.summary)
            {
                suppressions.push(extra);
            }
        }
    }
    let mut diagnostics = TopologyDiagnostics {
        kind: "arcscan-topology-diagnostics".into(),
        devices,
        run_summary: TopologyRunSummary::default(),
        suppressions,
        correlation_notes: notes,
    };
    fill_run_summary(&mut diagnostics, snapshot);
    diagnostics
}

pub fn absorb_failures(diagnostics: &mut TopologyDiagnostics, failures: &[TopologyDeviceFailure]) {
    for failure in failures {
        if diagnostics
            .devices
            .iter()
            .any(|device| device.target_ip == failure.ip)
        {
            continue;
        }
        diagnostics
            .devices
            .push(failed_device(&failure.ip, &failure.reason));
    }
    diagnostics
        .devices
        .sort_by(|a, b| a.target_ip.cmp(&b.target_ip));
}

pub fn export_diagnostics_json(diagnostics: &TopologyDiagnostics) -> String {
    let mut value = serde_json::to_value(diagnostics).unwrap_or(Value::Null);
    scrub_value(&mut value);
    let wrapped = serde_json::json!({
        "kind": "arcscan-topology-diagnostics",
        "warning": EXPORT_WARNING,
        "diagnostics": value,
    });
    serde_json::to_string_pretty(&wrapped).unwrap_or_else(|_| "{}".to_string())
}

fn device_from_view(
    view: &DeviceView,
    targets: &[TopologyTarget],
    trace: &CorrelationTrace,
) -> DeviceTopologyDiagnostics {
    let target_ip = view.target_ip.to_string();
    let inventory_device_id = view.inventory_hint.or_else(|| {
        targets
            .iter()
            .find(|target| target.ip == target_ip)
            .and_then(|target| target.device_id)
    });
    let display_name = view.sys_name.clone().or_else(|| {
        inventory_device_id.and_then(|id| {
            let label = inventory_label(targets, id);
            if label.starts_with("device ") {
                None
            } else {
                Some(label)
            }
        })
    });
    let coverage = mib_coverage(&view.probes);
    let lldp_state = neighbour_protocol_state(view, "lldp");
    let cdp_state = neighbour_protocol_state(view, "cdp");
    let all_relationships = relationships_for(view, &target_ip, targets, trace);
    let relationship_count = all_relationships.len();
    let unresolved_peers = all_relationships
        .iter()
        .filter(|link| link.to_device_id.is_none())
        .count();
    let relationships_omitted = relationship_count.saturating_sub(MAX_LISTED_RELATIONSHIPS);
    let relationships: Vec<_> = all_relationships
        .into_iter()
        .take(MAX_LISTED_RELATIONSHIPS)
        .collect();
    let mut suppressions: Vec<_> = trace
        .suppressions
        .iter()
        .filter(|item| item.target_ip == target_ip)
        .cloned()
        .collect();
    let mut notes: Vec<String> = trace
        .notes
        .iter()
        .filter(|note| note.target_ip == target_ip)
        .map(|note| note.summary.clone())
        .collect();
    notes.extend(label_notes(view));
    suppressions.extend(malformed_suppressions(view, inventory_device_id));
    let suppressions_omitted = suppressions.len().saturating_sub(MAX_LISTED_SUPPRESSIONS);
    let suppressions: Vec<_> = suppressions
        .into_iter()
        .take(MAX_LISTED_SUPPRESSIONS)
        .collect();

    let fdb = fdb_diag(view, targets, trace, &target_ip);
    let arp = arp_diag(view, trace, &target_ip);
    let vlan = vlan_diag(view, &coverage);
    let poe = poe_diag(view, &coverage);
    let lldp = neighbor_diag(view, trace, &target_ip, "lldp", lldp_state);
    let cdp = neighbor_diag(view, trace, &target_ip, "cdp", cdp_state);
    let port_mappings: Vec<PortMappingDiag> = relationships
        .iter()
        .filter_map(|link| link.port.clone())
        .take(MAX_PORT_DETAILS)
        .collect();
    let interfaces = InterfaceStats {
        count: view.interfaces.len(),
        up: view
            .interfaces
            .values()
            .filter(|iface| iface.is_up())
            .count(),
        with_name: view
            .interfaces
            .values()
            .filter(|iface| iface.name.is_some() || iface.alias.is_some() || iface.descr.is_some())
            .count(),
        with_speed: view
            .interfaces
            .values()
            .filter(|iface| iface.speed_mbps.is_some())
            .count(),
    };

    let mut device = DeviceTopologyDiagnostics {
        target_ip,
        inventory_device_id,
        display_name,
        snmp_status: SnmpStatus::Responded,
        failure_reason: None,
        sys_name: view.sys_name.clone(),
        mib_coverage: coverage,
        interfaces,
        lldp,
        cdp,
        fdb,
        arp,
        vlan,
        poe,
        relationships,
        relationship_count,
        relationships_omitted,
        unresolved_peers,
        suppressions,
        suppressions_omitted,
        port_mappings,
        notes,
        hints: Vec::new(),
        zero_link_explanation: None,
    };
    device.hints = hints_for(view, &device);
    device.zero_link_explanation = zero_link_explanation(view, &device);
    if let Some(model) = view.entity.model.as_deref() {
        device.notes.push(format!(
            "ENTITY-MIB reported model \"{model}\" from the first physical row. That is metadata, not a device classification."
        ));
    }
    device
}

fn failed_device(ip: &str, reason: &str) -> DeviceTopologyDiagnostics {
    let status = classify_failure(reason);
    let explanation = match status {
        SnmpStatus::Timeout => {
            Some("SNMP timed out. No topology tables were read from this device.".to_string())
        }
        SnmpStatus::AuthFailed => Some(
            "SNMP authentication failed. No topology tables were read from this device."
                .to_string(),
        ),
        SnmpStatus::Unreachable => {
            Some("ArcScan could not reach this device over SNMP.".to_string())
        }
        SnmpStatus::InvalidAddress => {
            Some("The address is not a valid IPv4 target, so no SNMP tables were read.".to_string())
        }
        _ => Some(
            "SNMP failed before topology tables could be read. The rest of the run continued."
                .to_string(),
        ),
    };
    let not_queried = |mib: &str| MibCoverage {
        mib: mib.to_string(),
        state: MibState::NotQueried,
        rows: 0,
        detail: Some("Not queried because SNMP did not succeed.".into()),
    };
    DeviceTopologyDiagnostics {
        target_ip: ip.to_string(),
        inventory_device_id: None,
        display_name: None,
        snmp_status: status,
        failure_reason: Some(redact_secrets(reason)),
        sys_name: None,
        mib_coverage: mib_names().into_iter().map(not_queried).collect(),
        interfaces: InterfaceStats {
            count: 0,
            up: 0,
            with_name: 0,
            with_speed: 0,
        },
        lldp: empty_neighbors(MibState::NotQueried),
        cdp: empty_neighbors(MibState::NotQueried),
        fdb: empty_fdb(MibState::NotQueried),
        arp: ArpDiag {
            state: MibState::NotQueried,
            entries: 0,
            fdb_corroborations: 0,
        },
        vlan: VlanDiag {
            state: MibState::NotQueried,
            pvid_ports: 0,
            access_ports: 0,
            trunk_ports: 0,
        },
        poe: PoeDiag {
            detection_state: MibState::NotQueried,
            wattage_state: MibState::NotQueried,
            enabled_ports: 0,
            ports_with_watts: 0,
            enabled_without_watts: 0,
        },
        relationships: Vec::new(),
        relationship_count: 0,
        relationships_omitted: 0,
        unresolved_peers: 0,
        suppressions: Vec::new(),
        suppressions_omitted: 0,
        port_mappings: Vec::new(),
        notes: Vec::new(),
        hints: Vec::new(),
        zero_link_explanation: explanation,
    }
}

fn classify_failure(reason: &str) -> SnmpStatus {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("did not answer") || lower.contains("timed out") || lower.contains("timeout")
    {
        SnmpStatus::Timeout
    } else if lower.contains("authentication failed") {
        SnmpStatus::AuthFailed
    } else if lower.contains("could not reach") || lower.contains("unreachable") {
        SnmpStatus::Unreachable
    } else if lower.contains("not a valid ipv4") {
        SnmpStatus::InvalidAddress
    } else {
        SnmpStatus::ProtocolError
    }
}

fn mib_names() -> [&'static str; 9] {
    [
        "IF-MIB",
        "LLDP-MIB",
        "CISCO-CDP-MIB",
        "BRIDGE-MIB",
        "Q-BRIDGE-MIB",
        "IP-MIB",
        "POWER-ETHERNET-MIB",
        "CISCO-POWER-ETHERNET-EXT-MIB",
        "ENTITY-MIB",
    ]
}

fn mib_coverage(probes: &[TableProbe]) -> Vec<MibCoverage> {
    mib_names()
        .into_iter()
        .map(|mib| {
            let rows: Vec<&TableProbe> = probes.iter().filter(|probe| probe.mib == mib).collect();
            let (state, total) = aggregate_mib(&rows);
            let detail = mib_detail(mib, &rows, state);
            MibCoverage {
                mib: mib.to_string(),
                state,
                rows: total,
                detail,
            }
        })
        .collect()
}

fn aggregate_mib(rows: &[&TableProbe]) -> (MibState, usize) {
    if rows.is_empty() {
        return (MibState::NotQueried, 0);
    }
    let queried: Vec<ProbeState> = rows
        .iter()
        .map(|probe| probe.state)
        .filter(|state| *state != ProbeState::NotQueried)
        .collect();
    let total = rows
        .iter()
        .filter(|probe| probe.state == ProbeState::Available)
        .map(|probe| probe.rows)
        .sum();
    if queried.is_empty() {
        return (MibState::NotQueried, 0);
    }
    let any_rows = queried.contains(&ProbeState::Available);
    let any_timeout = queried.contains(&ProbeState::TimedOut);
    let any_fail = queried.contains(&ProbeState::WalkFailed);
    let any_empty = queried.contains(&ProbeState::NoRows);
    let state = if any_rows && (any_timeout || any_fail) {
        MibState::Partial
    } else if any_rows {
        MibState::Available
    } else if (any_empty && (any_timeout || any_fail)) || (any_timeout && any_fail) {
        MibState::Partial
    } else if any_timeout {
        MibState::TimedOut
    } else if any_fail {
        MibState::WalkFailed
    } else if any_empty {
        MibState::NoRows
    } else {
        MibState::NotQueried
    };
    (state, total)
}

fn mib_detail(mib: &str, rows: &[&TableProbe], state: MibState) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    if mib == "LLDP-MIB" {
        let remote = rows.iter().find(|probe| probe.table == "lldpRemChassisId");
        let local_rows: usize = rows
            .iter()
            .filter(|probe| probe.table.starts_with("lldpLoc"))
            .map(|probe| probe.rows)
            .sum();
        return match remote.map(|probe| probe.state) {
            Some(ProbeState::NoRows) => Some(format!(
                "Remote neighbour table returned no rows. Local port table returned {local_rows} rows."
            )),
            Some(ProbeState::WalkFailed) => {
                Some("LLDP remote neighbour walk failed. Neighbours were not inferred.".into())
            }
            Some(ProbeState::TimedOut) => {
                Some("LLDP remote neighbour walk timed out. Neighbours were not inferred.".into())
            }
            Some(ProbeState::Available) if state == MibState::Partial => {
                Some("LLDP returned neighbours, but at least one LLDP table walk failed.".into())
            }
            _ => None,
        };
    }
    if mib == "ENTITY-MIB" && state == MibState::Partial {
        return Some(
            "ENTITY-MIB answered only in part. The first model string is metadata, not a classified device."
                .into(),
        );
    }
    if state == MibState::Partial {
        return Some(
            "At least one table in this MIB answered and another walk failed or timed out.".into(),
        );
    }
    None
}

fn coverage_state(coverage: &[MibCoverage], mib: &str) -> MibState {
    coverage
        .iter()
        .find(|item| item.mib == mib)
        .map(|item| item.state)
        .unwrap_or(MibState::NotQueried)
}

fn same_identity(left: Option<&str>, right: Option<&str>) -> bool {
    match (
        left.map(str::trim).filter(|s| !s.is_empty()),
        right.map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (Some(a), Some(b)) => {
            if let (Some(ma), Some(mb)) = (normalize_mac(a), normalize_mac(b)) {
                ma == mb
            } else {
                a.eq_ignore_ascii_case(b)
            }
        }
        _ => false,
    }
}

fn only_link_on_port<'a>(on_port: &[&'a ObservedLink]) -> Option<&'a ObservedLink> {
    if on_port.len() == 1 {
        on_port.first().copied()
    } else {
        None
    }
}

fn lldp_link_for<'a>(
    links: &[&'a ObservedLink],
    neighbor: &LldpNeighbor,
) -> Option<&'a ObservedLink> {
    let on_port: Vec<_> = links
        .iter()
        .copied()
        .filter(|link| link.raw_port == neighbor.local_port_num)
        .collect();
    on_port
        .iter()
        .copied()
        .find(|link| {
            same_identity(
                link.management_address.as_deref(),
                neighbor.management_address.as_deref(),
            ) || same_identity(
                link.chassis_or_mac.as_deref(),
                neighbor.chassis_id.as_deref(),
            )
        })
        .or_else(|| only_link_on_port(&on_port))
}

fn cdp_link_for<'a>(
    links: &[&'a ObservedLink],
    neighbor: &CdpNeighbor,
) -> Option<&'a ObservedLink> {
    let on_port: Vec<_> = links
        .iter()
        .copied()
        .filter(|link| link.raw_port == neighbor.if_index)
        .collect();
    on_port
        .iter()
        .copied()
        .find(|link| {
            same_identity(
                link.management_address.as_deref(),
                neighbor.address.as_deref(),
            ) || same_identity(link.sys_name.as_deref(), neighbor.device_id.as_deref())
        })
        .or_else(|| only_link_on_port(&on_port))
}

/// Neighbour-protocol state follows the remote-neighbour table. Local LLDP
/// port rows can make the MIB aggregate `Available` while the remote table is
/// empty; that is "no neighbours", not a neighbour claim.
fn neighbour_protocol_state(view: &DeviceView, protocol: &str) -> MibState {
    let table = if protocol == "lldp" {
        "lldpRemChassisId"
    } else {
        "cdpCacheDeviceId"
    };
    let count = if protocol == "lldp" {
        view.lldp_neighbors.len()
    } else {
        view.cdp_neighbors.len()
    };
    if let Some(probe) = view.probes.iter().find(|probe| probe.table == table) {
        return match probe.state {
            ProbeState::Available => MibState::Available,
            ProbeState::NoRows => MibState::NoRows,
            ProbeState::TimedOut => MibState::TimedOut,
            ProbeState::WalkFailed => MibState::WalkFailed,
            ProbeState::NotQueried => MibState::NotQueried,
        };
    }
    if count > 0 {
        MibState::Available
    } else {
        MibState::NotQueried
    }
}

fn neighbor_diag(
    view: &DeviceView,
    trace: &CorrelationTrace,
    source_ip: &str,
    protocol: &str,
    state: MibState,
) -> NeighborProtoDiag {
    let links: Vec<&ObservedLink> = trace
        .links
        .iter()
        .filter(|link| link.source_ip == source_ip && link.protocol == protocol)
        .collect();
    let neighbours = if protocol == "lldp" {
        view.lldp_neighbors
            .iter()
            .take(MAX_PORT_DETAILS)
            .map(|neighbor| {
                let matched = lldp_link_for(&links, neighbor);
                let resolution = matched
                    .map(|link| link.resolution.as_str())
                    .unwrap_or("unresolved");
                let resolved = matched.and_then(|link| link.to_device_id);
                NeighborObservation {
                    local_port: view.port_name(view.resolve_if_index(neighbor.local_port_num)),
                    remote_port: neighbor.port_id.clone().or(neighbor.port_desc.clone()),
                    chassis_id: neighbor.chassis_id.clone(),
                    management_address: neighbor.management_address.clone(),
                    sys_name: neighbor.sys_name.clone(),
                    resolved_device_id: resolved,
                    resolution: resolution.to_string(),
                }
            })
            .collect()
    } else {
        view.cdp_neighbors
            .iter()
            .take(MAX_PORT_DETAILS)
            .map(|neighbor| {
                let matched = cdp_link_for(&links, neighbor);
                let resolution = matched
                    .map(|link| link.resolution.as_str())
                    .unwrap_or("unresolved");
                NeighborObservation {
                    local_port: view.port_name(view.resolve_if_index(neighbor.if_index)),
                    remote_port: neighbor.device_port.clone(),
                    chassis_id: None,
                    management_address: neighbor.address.clone(),
                    sys_name: neighbor.device_id.clone(),
                    resolved_device_id: matched.and_then(|link| link.to_device_id),
                    resolution: resolution.to_string(),
                }
            })
            .collect()
    };
    let count = if protocol == "lldp" {
        view.lldp_neighbors.len()
    } else {
        view.cdp_neighbors.len()
    };
    let resolved = links
        .iter()
        .filter(|link| link.to_device_id.is_some())
        .count();
    NeighborProtoDiag {
        state,
        neighbour_count: count,
        resolved,
        unresolved: count.saturating_sub(resolved),
        neighbours,
    }
}

fn fdb_diag(
    view: &DeviceView,
    targets: &[TopologyTarget],
    trace: &CorrelationTrace,
    source_ip: &str,
) -> FdbDiag {
    let own = view.own_macs();
    let mut by_port: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    let mut unicast = BTreeSet::new();
    for entry in &view.fdb {
        if !is_unicast_mac(&entry.mac) || own.contains(&entry.mac) {
            continue;
        }
        unicast.insert(entry.mac.as_str());
        by_port
            .entry(entry.if_index)
            .or_default()
            .push(entry.mac.as_str());
    }
    let matched_inventory = unicast
        .iter()
        .filter(|mac| {
            targets.iter().any(|target| {
                target
                    .mac
                    .as_deref()
                    .and_then(crate::collect::normalize_mac)
                    .as_deref()
                    == Some(**mac)
            })
        })
        .count();
    let mut single = 0usize;
    let mut multi = 0usize;
    let mut ports = Vec::new();
    for (port, macs) in &by_port {
        let unique: BTreeSet<&str> = macs.iter().copied().collect();
        if unique.len() >= 2 {
            multi += 1;
        } else if unique.len() == 1 {
            single += 1;
        }
        let suppressed = trace.suppressions.iter().any(|item| {
            item.target_ip == source_ip
                && item.port_label.as_deref() == Some(view.port_name(*port).as_str())
                && (item.reason == "multi-mac-uplink" || item.reason == "trunk-single-mac")
        });
        let promoted = trace.links.iter().any(|link| {
            link.source_ip == source_ip && link.protocol == "fdb" && link.raw_port == *port
        });
        if !suppressed && !promoted && unique.len() < 2 {
            continue;
        }
        let outcome = if unique.len() >= 2 {
            "suppressed-uplink"
        } else if promoted {
            "strong-link"
        } else if suppressed {
            "suppressed"
        } else {
            "observed"
        };
        let summary = if unique.len() >= 2 {
            format!(
                "Port {} learned {} relevant unicast MACs. The relationship was suppressed because this resembles an uplink or trunk rather than a directly attached endpoint.",
                view.port_name(*port),
                unique.len()
            )
        } else if promoted {
            format!(
                "Port {} learned one relevant MAC and was eligible for a strong FDB link.",
                view.port_name(*port)
            )
        } else {
            format!(
                "Port {} learned {} relevant MAC{}.",
                view.port_name(*port),
                unique.len(),
                if unique.len() == 1 { "" } else { "s" }
            )
        };
        ports.push(FdbPortDiag {
            port_label: view.port_name(*port),
            raw_port: *port,
            relevant_macs: unique.len(),
            matched_inventory: unique
                .iter()
                .filter(|mac| {
                    targets.iter().any(|target| {
                        target
                            .mac
                            .as_deref()
                            .and_then(crate::collect::normalize_mac)
                            .as_deref()
                            == Some(**mac)
                    })
                })
                .count(),
            outcome: outcome.into(),
            summary,
        });
    }
    ports.sort_by(|a, b| {
        b.relevant_macs
            .cmp(&a.relevant_macs)
            .then(a.raw_port.cmp(&b.raw_port))
    });
    let ports_omitted = ports.len().saturating_sub(MAX_PORT_DETAILS);
    ports.truncate(MAX_PORT_DETAILS);
    let strong_links = trace
        .links
        .iter()
        .filter(|link| link.source_ip == source_ip && link.protocol == "fdb")
        .count();
    let uplink_suppressions = trace
        .suppressions
        .iter()
        .filter(|item| item.target_ip == source_ip && item.reason == "multi-mac-uplink")
        .count();
    let probed_rows: usize = view
        .probes
        .iter()
        .filter(|probe| probe.table == "dot1dTpFdbPort" || probe.table == "dot1qTpFdbPort")
        .map(|probe| probe.rows)
        .sum();
    let total_rows = if view.probes.is_empty() {
        view.fdb.len()
    } else {
        probed_rows
    };
    let state = if !view.probes.is_empty() {
        let fdb_probes: Vec<&TableProbe> = view
            .probes
            .iter()
            .filter(|probe| probe.table == "dot1dTpFdbPort" || probe.table == "dot1qTpFdbPort")
            .collect();
        aggregate_mib(&fdb_probes).0
    } else if view.fdb.is_empty() {
        MibState::NoRows
    } else {
        MibState::Available
    };
    FdbDiag {
        state,
        total_rows,
        unicast_macs: unicast.len(),
        matched_inventory,
        unresolved_macs: unicast.len().saturating_sub(matched_inventory),
        single_mac_ports: single,
        multi_mac_ports: multi,
        strong_links,
        uplink_suppressions,
        ports_omitted,
        ports,
    }
}

fn arp_diag(view: &DeviceView, trace: &CorrelationTrace, source_ip: &str) -> ArpDiag {
    let state = if view.probes.is_empty() {
        if view.arp.is_empty() {
            MibState::NoRows
        } else {
            MibState::Available
        }
    } else {
        let rows: Vec<&TableProbe> = view
            .probes
            .iter()
            .filter(|probe| probe.mib == "IP-MIB")
            .collect();
        aggregate_mib(&rows).0
    };
    let corroborations = trace
        .links
        .iter()
        .filter(|link| link.source_ip == source_ip && link.arp_ip.is_some())
        .count();
    ArpDiag {
        state,
        entries: view.arp.len(),
        fdb_corroborations: corroborations,
    }
}

fn vlan_diag(view: &DeviceView, coverage: &[MibCoverage]) -> VlanDiag {
    let mut access = 0usize;
    let mut trunk = 0usize;
    let ports: BTreeSet<u32> = view
        .pvid
        .keys()
        .copied()
        .chain(view.tagged.keys().copied())
        .collect();
    for port in ports {
        let (vlan, _, _) = view.vlan_for_port(port);
        if vlan.as_deref() == Some("trunk") {
            trunk += 1;
        } else if vlan.is_some() {
            access += 1;
        }
    }
    let state = if !view.pvid.is_empty() || !view.tagged.is_empty() {
        MibState::Available
    } else {
        coverage_state(coverage, "Q-BRIDGE-MIB")
    };
    VlanDiag {
        state,
        pvid_ports: view.pvid.len(),
        access_ports: access,
        trunk_ports: trunk,
    }
}

fn poe_diag(view: &DeviceView, coverage: &[MibCoverage]) -> PoeDiag {
    let mut enabled = 0usize;
    let mut with_watts = 0usize;
    let mut enabled_without = 0usize;
    for iface in view.interfaces.values() {
        if let Some(poe) = &iface.poe {
            if poe.enabled {
                enabled += 1;
                if poe.watts.is_some() {
                    with_watts += 1;
                } else {
                    enabled_without += 1;
                }
            }
        }
    }
    PoeDiag {
        detection_state: if enabled > 0 {
            MibState::Available
        } else {
            coverage_state(coverage, "POWER-ETHERNET-MIB")
        },
        wattage_state: if with_watts > 0 {
            MibState::Available
        } else {
            coverage_state(coverage, "CISCO-POWER-ETHERNET-EXT-MIB")
        },
        enabled_ports: enabled,
        ports_with_watts: with_watts,
        enabled_without_watts: enabled_without,
    }
}

fn relationships_for(
    view: &DeviceView,
    source_ip: &str,
    targets: &[TopologyTarget],
    trace: &CorrelationTrace,
) -> Vec<RelationshipDiag> {
    trace
        .links
        .iter()
        .filter(|link| link.source_ip == source_ip)
        .map(|link| RelationshipDiag {
            protocol: link.protocol.clone(),
            confidence: link.confidence.clone(),
            from_device_id: link.from_device_id,
            to_device_id: link.to_device_id,
            to_unresolved_id: link.to_unresolved_id.clone(),
            from_port: link.from_port.clone(),
            to_port: link.to_port.clone(),
            resolution: link.resolution.clone(),
            why: why_link(link, targets),
            port: Some(explain_port(view, link.raw_port, &link.port_role)),
        })
        .collect()
}

fn why_link(link: &ObservedLink, targets: &[TopologyTarget]) -> String {
    let local = link.from_port.as_deref().unwrap_or("unknown port");
    let remote = link.to_port.as_deref().unwrap_or("unknown");
    let peer = link
        .to_device_id
        .map(|id| inventory_label(targets, id))
        .unwrap_or_else(|| "an unresolved neighbour".into());
    match link.protocol.as_str() {
        "lldp" if link.resolution == "management-ip" => format!(
            "LLDP reported chassis {}. Management IP {} matched inventory device {peer}. Local port {local}. Remote port {remote}.",
            link.chassis_or_mac.as_deref().unwrap_or("unknown"),
            link.management_address.as_deref().unwrap_or("unknown"),
        ),
        "lldp" if link.resolution == "chassis-mac" => format!(
            "LLDP reported chassis MAC {}, which matched inventory device {peer}. Local port {local}. Remote port {remote}.",
            link.chassis_or_mac.as_deref().unwrap_or("unknown"),
        ),
        "lldp" => format!(
            "LLDP reported chassis {} and sysName {}. Neither management IP nor chassis MAC matched inventory, so the neighbour stays unresolved. Local port {local}. Remote port {remote}.",
            link.chassis_or_mac.as_deref().unwrap_or("unknown"),
            link.sys_name.as_deref().unwrap_or("unknown"),
        ),
        "cdp" if link.resolution == "management-ip" => format!(
            "CDP reported device id {} at {}, which matched inventory device {peer}. Local port {local}. Remote port {remote}.",
            link.sys_name.as_deref().unwrap_or("unknown"),
            link.management_address.as_deref().unwrap_or("unknown"),
        ),
        "cdp" => format!(
            "CDP reported device id {}. No management address proved an inventory device, so the neighbour stays unresolved. Local port {local}. Remote port {remote}.",
            link.sys_name.as_deref().unwrap_or("unknown"),
        ),
        "fdb" if link.arp_ip.is_some() && link.to_device_id.is_some() => format!(
            "Switch FDB learned exactly one relevant inventory MAC ({}) on port {local}. MAC belongs to {peer}. ARP independently associates that MAC with {}.",
            link.chassis_or_mac.as_deref().unwrap_or("unknown"),
            link.arp_ip.as_deref().unwrap_or("unknown"),
        ),
        "fdb" if link.to_device_id.is_some() => format!(
            "Switch FDB learned exactly one relevant inventory MAC ({}) on port {local}. MAC belongs to {peer}.",
            link.chassis_or_mac.as_deref().unwrap_or("unknown"),
        ),
        "fdb" => format!(
            "Exactly one relevant unicast MAC ({}) was learned on access port {local}. It does not match an inventory device, so the peer stays unresolved.",
            link.chassis_or_mac.as_deref().unwrap_or("unknown"),
        ),
        _ => format!(
            "{} evidence on {local} produced a {} relationship.",
            link.protocol, link.confidence
        ),
    }
}

fn label_notes(view: &DeviceView) -> Vec<String> {
    let mut notes = Vec::new();
    for rejected in &view.rejected_labels {
        let shown = view
            .iface(rejected.if_index)
            .map(|iface| iface.display_name())
            .unwrap_or_else(|| rejected.if_index.to_string());
        if shown != rejected.if_index.to_string() {
            notes.push(format!(
                "{} on ifIndex {} was not a printable port name. {} was used instead.",
                rejected.field, rejected.if_index, shown
            ));
        }
    }
    notes
}

fn malformed_suppressions(view: &DeviceView, inventory_device_id: Option<i64>) -> Vec<Suppression> {
    let mut out = Vec::new();
    for rejected in &view.rejected_labels {
        let shown = view
            .iface(rejected.if_index)
            .map(|iface| iface.display_name())
            .unwrap_or_else(|| rejected.if_index.to_string());
        if shown == rejected.if_index.to_string() {
            out.push(Suppression {
                target_ip: view.target_ip.to_string(),
                inventory_device_id,
                reason: "malformed-port-label".into(),
                summary: format!(
                    "{} on ifIndex {} was not a printable port name, so the numeric index is shown instead.",
                    rejected.field, rejected.if_index
                ),
                port_label: Some(shown),
                mac_count: None,
            });
        }
    }
    out
}

fn hints_for(view: &DeviceView, device: &DeviceTopologyDiagnostics) -> Vec<String> {
    let mut hints = Vec::new();
    let lldp_mib = coverage_state(&device.mib_coverage, "LLDP-MIB");
    let cdp_mib = coverage_state(&device.mib_coverage, "CISCO-CDP-MIB");
    let bridge = coverage_state(&device.mib_coverage, "BRIDGE-MIB");
    let if_mib = coverage_state(&device.mib_coverage, "IF-MIB");
    let fdb_usable = device.fdb.unicast_macs > 0 || device.fdb.state == MibState::Available;
    if device.lldp.state == MibState::NoRows && fdb_usable {
        hints.push(
            "LLDP may be disabled on this switch. FDB topology is still available.".to_string(),
        );
    }
    let unavailable = |state: MibState| {
        matches!(
            state,
            MibState::WalkFailed | MibState::NotQueried | MibState::TimedOut
        )
    };
    if if_mib == MibState::Available
        && unavailable(bridge)
        && unavailable(lldp_mib)
        && unavailable(cdp_mib)
    {
        hints.push(
            "This device exposes interface data but no usable neighbour or bridge topology tables."
                .into(),
        );
    }
    if device.fdb.multi_mac_ports >= 2 {
        hints.push(
            "Several ports look like uplinks or downstream switches. Direct endpoint links were intentionally suppressed."
                .into(),
        );
    }
    if view.lldp_neighbors.is_empty()
        && matches!(device.lldp.state, MibState::WalkFailed | MibState::TimedOut)
        && device.fdb.strong_links > 0
    {
        hints.push(
            "The LLDP walk failed. FDB links that passed the access-port rule are still listed."
                .into(),
        );
    }
    hints
}

fn zero_link_explanation(view: &DeviceView, device: &DeviceTopologyDiagnostics) -> Option<String> {
    if device.relationship_count > 0 {
        return None;
    }
    let neighbour_count = view.lldp_neighbors.len() + view.cdp_neighbors.len();
    if neighbour_count > 0 {
        return Some(format!(
            "LLDP/CDP reported {neighbour_count} neighbours, but none could be kept after the existing safety checks."
        ));
    }
    if device.fdb.unicast_macs > 0
        && device.fdb.multi_mac_ports > 0
        && device.fdb.single_mac_ports == 0
    {
        return Some(
            "FDB is available, but all learned MACs were on multi-MAC ports that look like uplinks."
                .into(),
        );
    }
    let if_mib = coverage_state(&device.mib_coverage, "IF-MIB");
    let bridge = coverage_state(&device.mib_coverage, "BRIDGE-MIB");
    let lldp = coverage_state(&device.mib_coverage, "LLDP-MIB");
    let cdp = coverage_state(&device.mib_coverage, "CISCO-CDP-MIB");
    let missing = |state: MibState| {
        matches!(
            state,
            MibState::WalkFailed | MibState::NotQueried | MibState::TimedOut | MibState::NoRows
        )
    };
    if if_mib == MibState::Available
        && missing(bridge)
        && missing(lldp)
        && missing(cdp)
        && device.fdb.total_rows == 0
    {
        return Some("IF-MIB responded, but neighbour and bridge topology tables did not.".into());
    }
    if matches!(lldp, MibState::NoRows | MibState::NotQueried)
        && matches!(cdp, MibState::NoRows | MibState::NotQueried)
        && device.fdb.unicast_macs == 0
    {
        return Some(
            "SNMP responded, but LLDP/CDP returned no neighbours and BRIDGE-MIB exposed no usable FDB entries."
                .into(),
        );
    }
    Some("SNMP responded, but no relationship passed the existing topology safety rules.".into())
}

fn empty_neighbors(state: MibState) -> NeighborProtoDiag {
    NeighborProtoDiag {
        state,
        neighbour_count: 0,
        resolved: 0,
        unresolved: 0,
        neighbours: Vec::new(),
    }
}

fn empty_fdb(state: MibState) -> FdbDiag {
    FdbDiag {
        state,
        total_rows: 0,
        unicast_macs: 0,
        matched_inventory: 0,
        unresolved_macs: 0,
        single_mac_ports: 0,
        multi_mac_ports: 0,
        strong_links: 0,
        uplink_suppressions: 0,
        ports_omitted: 0,
        ports: Vec::new(),
    }
}

fn fill_run_summary(diagnostics: &mut TopologyDiagnostics, snapshot: &TopologySnapshot) {
    let responding = diagnostics
        .devices
        .iter()
        .filter(|device| device.snmp_status == SnmpStatus::Responded)
        .count();
    let failed = diagnostics
        .devices
        .iter()
        .filter(|device| device.snmp_status != SnmpStatus::Responded)
        .count();
    let partial = diagnostics
        .devices
        .iter()
        .filter(|device| {
            device.snmp_status == SnmpStatus::Responded
                && device.mib_coverage.iter().any(|mib| {
                    matches!(
                        mib.state,
                        MibState::Partial | MibState::WalkFailed | MibState::TimedOut
                    )
                })
        })
        .count();
    let mut confirmed = 0usize;
    let mut strong = 0usize;
    let mut inferred = 0usize;
    let mut fdb_relationships = 0usize;
    for connection in &snapshot.connections {
        if connection.kind == "wan" {
            continue;
        }
        match connection.confidence {
            TopologyConfidence::Confirmed => confirmed += 1,
            TopologyConfidence::Strong => strong += 1,
            TopologyConfidence::Inferred => inferred += 1,
        }
        if connection.protocol == "fdb" {
            fdb_relationships += 1;
        }
    }
    diagnostics.run_summary = TopologyRunSummary {
        devices_queried: diagnostics.devices.len(),
        devices_responding: responding,
        devices_failed: failed,
        lldp_cdp_neighbours: diagnostics
            .devices
            .iter()
            .map(|device| device.lldp.neighbour_count + device.cdp.neighbour_count)
            .sum(),
        fdb_relationships,
        confirmed_links: confirmed,
        strong_links: strong,
        inferred_links: inferred,
        unresolved_neighbours: snapshot.unknown_nodes.len(),
        suppressed_candidates: diagnostics.suppressions.len(),
        partial_snmp_devices: partial,
    };
}

fn scrub_value(value: &mut Value) {
    match value {
        Value::String(text) => *text = scrub_text(text),
        Value::Array(items) => items.iter_mut().for_each(scrub_value),
        Value::Object(map) => {
            map.retain(|key, _| !secret_key(key));
            map.values_mut().for_each(scrub_value);
        }
        _ => {}
    }
}

fn secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "community"
            | "username"
            | "authpassword"
            | "privpassword"
            | "auth_password"
            | "priv_password"
            | "authpass"
            | "privpass"
    )
}

fn scrub_text(text: &str) -> String {
    let mut out = text.to_string();
    for needle in [
        "username ",
        "username=",
        "auth password ",
        "privacy password ",
        "auth_password ",
        "priv_password ",
        "authPass ",
        "privPass ",
        "community ",
        "community=",
    ] {
        out = redact_following(&out, needle);
    }
    redact_secrets(&out)
}

fn redact_following(text: &str, needle: &str) -> String {
    let mut out = text.to_string();
    if let Some(idx) = out.to_ascii_lowercase().find(&needle.to_ascii_lowercase()) {
        let rest = &out[idx + needle.len()..];
        let cut = rest
            .find(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        let start = idx + needle.len();
        out.replace_range(start..start + cut, "[redacted]");
    }
    out
}

/// Confidence string stored on an observed link. Matches the snapshot vocabulary.
pub fn confidence_name(confidence: TopologyConfidence) -> &'static str {
    confidence.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{
        ArpEntry, CdpNeighbor, FdbEntry, Iface, LldpNeighbor, ProbeState, RejectedLabel, TableProbe,
    };
    use crate::correlate::{correlate, correlate_detailed};
    use crate::model::{EdgeHint, PoeInfo, TopologyConfidence, TopologySnapshot};
    use std::collections::BTreeSet;
    use std::net::Ipv4Addr;

    fn target(id: i64, ip: &str, mac: &str, name: &str) -> TopologyTarget {
        TopologyTarget {
            ip: ip.into(),
            mac: Some(mac.into()),
            device_id: Some(id),
            hostname: Some(name.into()),
            detected_name: Some(name.into()),
        }
    }

    fn iface(index: u32, name: &str) -> Iface {
        Iface {
            index,
            name: Some(name.into()),
            descr: None,
            alias: None,
            mac: None,
            if_type: Some(6),
            admin_status: Some(1),
            oper_status: Some(1),
            speed_mbps: Some(1000),
            poe: None,
        }
    }

    fn switch() -> DeviceView {
        let mut view = DeviceView::new(Ipv4Addr::new(192, 168, 60, 2), Some(2));
        view.sys_name = Some("NETGEAR-SW1".into());
        view.bridge_address = Some("00:1A:2B:00:00:02".into());
        view.chassis_id = Some("00:1A:2B:00:00:02".into());
        for idx in [7u32, 12, 20, 24] {
            view.interfaces
                .insert(idx, iface(idx, &format!("Port {idx}")));
            view.pvid.insert(idx, 10);
        }
        view
    }

    fn targets() -> Vec<TopologyTarget> {
        vec![
            target(1, "192.168.60.1", "00:20:AA:00:00:01", "FW1"),
            target(2, "192.168.60.2", "00:1A:2B:00:00:02", "NETGEAR-SW1"),
            target(3, "192.168.60.12", "00:1A:2B:00:00:12", "AP-Lobby"),
            target(4, "192.168.60.20", "00:11:32:00:00:20", "BC-NAS1"),
            target(5, "192.168.60.50", "AA:BB:CC:00:00:50", "SUSAN-MINI"),
        ]
    }

    fn device<'a>(diag: &'a TopologyDiagnostics, ip: &str) -> &'a DeviceTopologyDiagnostics {
        diag.devices
            .iter()
            .find(|item| item.target_ip == ip)
            .unwrap()
    }

    #[test]
    fn lldp_confirmed_relationship_explains_management_ip() {
        let mut core = switch();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 12,
            chassis_id: Some("00:1A:2B:00:00:12".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth0".into()),
            port_desc: None,
            sys_name: Some("AP-Lobby".into()),
            sys_desc: None,
            management_address: Some("192.168.60.12".into()),
        });
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        let link = snap
            .connections
            .iter()
            .find(|c| c.to_device_id == Some(3))
            .unwrap();
        assert_eq!(link.confidence, TopologyConfidence::Confirmed);
        assert_eq!(link.protocol, "lldp");
        let why = &device(&diag, "192.168.60.2").relationships[0].why;
        assert!(why.contains("Management IP 192.168.60.12"), "{why}");
        assert!(why.contains("AP-Lobby"), "{why}");
        assert!(why.contains("eth0"), "{why}");
    }

    #[test]
    fn cdp_confirmed_relationship_explains_address() {
        let mut core = switch();
        core.cdp_neighbors.push(CdpNeighbor {
            if_index: 20,
            device_id: Some("BC-NAS1".into()),
            device_port: Some("eth0".into()),
            platform: None,
            address: Some("192.168.60.20".into()),
            native_vlan: Some(10),
        });
        let (_snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        let why = &device(&diag, "192.168.60.2").relationships[0].why;
        assert!(why.contains("CDP reported"), "{why}");
        assert!(why.contains("192.168.60.20"), "{why}");
        assert!(why.contains("BC-NAS1"), "{why}");
    }

    #[test]
    fn single_mac_fdb_is_strong_and_arp_corroborates() {
        let mut core = switch();
        core.fdb.push(FdbEntry {
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: 7,
            vlan: Some(10),
        });
        core.arp.push(ArpEntry {
            ip: "192.168.60.50".into(),
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: Some(7),
        });
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        assert_eq!(snap.connections[0].confidence, TopologyConfidence::Strong);
        assert_eq!(snap.connections[0].protocol, "fdb");
        let dev = device(&diag, "192.168.60.2");
        assert!(
            dev.relationships[0].why.contains("ARP independently"),
            "{}",
            dev.relationships[0].why
        );
        assert_eq!(dev.arp.fdb_corroborations, 1);
        assert_eq!(dev.fdb.strong_links, 1);
        assert_eq!(dev.fdb.single_mac_ports, 1);
    }

    #[test]
    fn multi_mac_fdb_port_is_suppressed_without_fake_endpoints() {
        let mut core = switch();
        for last in 1..15u8 {
            core.fdb.push(FdbEntry {
                mac: format!("AA:AA:AA:00:00:{last:02X}"),
                if_index: 24,
                vlan: Some(10),
            });
        }
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        assert!(snap.connections.is_empty());
        let dev = device(&diag, "192.168.60.2");
        let suppression = dev
            .suppressions
            .iter()
            .find(|item| item.reason == "multi-mac-uplink")
            .unwrap();
        assert_eq!(suppression.mac_count, Some(14));
        assert!(
            suppression.summary.contains("uplink or trunk"),
            "{}",
            suppression.summary
        );
        assert!(dev
            .zero_link_explanation
            .as_deref()
            .unwrap()
            .contains("multi-MAC"));
        assert!(dev.fdb.ports.len() <= MAX_PORT_DETAILS);
    }

    #[test]
    fn hostname_only_lldp_stays_unresolved() {
        let mut core = switch();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 20,
            chassis_id: Some("DE:AD:BE:EF:00:99".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth0".into()),
            port_desc: None,
            sys_name: Some("AP-Lobby".into()),
            sys_desc: None,
            management_address: None,
        });
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        assert_eq!(snap.connections[0].to_device_id, None);
        assert_eq!(
            snap.connections[0].confidence,
            TopologyConfidence::Confirmed
        );
        let dev = device(&diag, "192.168.60.2");
        assert!(dev
            .suppressions
            .iter()
            .any(|item| { item.reason == "hostname-only" && item.summary.contains("AP-Lobby") }));
        assert!(dev.relationships[0].why.contains("stays unresolved"));
    }

    #[test]
    fn conflicting_ip_and_mac_is_explained_without_changing_the_link() {
        let mut core = switch();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 12,
            chassis_id: Some("00:11:32:00:00:20".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth0".into()),
            port_desc: None,
            sys_name: Some("BC-NAS1".into()),
            sys_desc: None,
            management_address: Some("192.168.60.12".into()),
        });
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        assert_eq!(snap.connections[0].to_device_id, Some(3));
        assert_eq!(
            snap.connections[0].confidence,
            TopologyConfidence::Confirmed
        );
        assert!(diag.correlation_notes.iter().any(|note| {
            note.summary.contains("keeps the management-IP match")
                && note.summary.contains("BC-NAS1")
        }));
    }

    #[test]
    fn self_loop_is_suppressed() {
        let mut core = switch();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 24,
            chassis_id: Some("00:1A:2B:00:00:02".into()),
            chassis_subtype: Some(4),
            port_id: Some("Port 1".into()),
            port_desc: None,
            sys_name: Some("NETGEAR-SW1".into()),
            sys_desc: None,
            management_address: Some("192.168.60.2".into()),
        });
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        let link = snap
            .connections
            .iter()
            .find(|connection| connection.from_device_id == Some(2))
            .expect("self-loop evidence stays in the snapshot");
        assert_eq!(link.to_device_id, Some(2));
        assert_eq!(link.confidence, TopologyConfidence::Confirmed);
        assert!(device(&diag, "192.168.60.2")
            .suppressions
            .iter()
            .any(|item| item.reason == "self-loop"
                && item.summary.contains("not a canonical topology connection")));
    }

    #[test]
    fn empty_remote_neighbour_table_is_no_neighbours_when_local_tables_answer() {
        let mut core = switch();
        core.probes.push(TableProbe {
            mib: "LLDP-MIB",
            table: "lldpLocPortId",
            state: ProbeState::Available,
            rows: 4,
        });
        core.probes.push(TableProbe {
            mib: "LLDP-MIB",
            table: "lldpRemChassisId",
            state: ProbeState::NoRows,
            rows: 0,
        });
        core.probes.push(TableProbe {
            mib: "CISCO-CDP-MIB",
            table: "cdpCacheAddress",
            state: ProbeState::Available,
            rows: 2,
        });
        core.probes.push(TableProbe {
            mib: "CISCO-CDP-MIB",
            table: "cdpCacheDeviceId",
            state: ProbeState::NoRows,
            rows: 0,
        });
        let (_snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        let dev = device(&diag, "192.168.60.2");
        let lldp_mib = dev
            .mib_coverage
            .iter()
            .find(|item| item.mib == "LLDP-MIB")
            .unwrap();
        let cdp_mib = dev
            .mib_coverage
            .iter()
            .find(|item| item.mib == "CISCO-CDP-MIB")
            .unwrap();
        assert_eq!(lldp_mib.state, MibState::Available);
        assert_eq!(cdp_mib.state, MibState::Available);
        assert_eq!(dev.lldp.state, MibState::NoRows);
        assert_eq!(dev.cdp.state, MibState::NoRows);
        assert_eq!(dev.lldp.neighbour_count, 0);
        assert_eq!(dev.cdp.neighbour_count, 0);
    }

    #[test]
    fn neighbours_on_the_same_port_keep_their_own_resolution() {
        let mut core = switch();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 12,
            chassis_id: Some("00:1A:2B:00:00:12".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth0".into()),
            port_desc: None,
            sys_name: Some("AP-Lobby".into()),
            sys_desc: None,
            management_address: Some("192.168.60.12".into()),
        });
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 12,
            chassis_id: Some("DE:AD:BE:EF:00:11".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth1".into()),
            port_desc: None,
            sys_name: Some("BC-NAS1".into()),
            sys_desc: None,
            management_address: None,
        });
        core.cdp_neighbors.push(CdpNeighbor {
            if_index: 20,
            device_id: Some("BC-NAS1".into()),
            device_port: Some("eth0".into()),
            platform: None,
            address: Some("192.168.60.20".into()),
            native_vlan: Some(10),
        });
        core.cdp_neighbors.push(CdpNeighbor {
            if_index: 20,
            device_id: Some("AP-Lobby".into()),
            device_port: Some("eth1".into()),
            platform: None,
            address: None,
            native_vlan: None,
        });
        let (_snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        let dev = device(&diag, "192.168.60.2");
        let resolved = dev
            .lldp
            .neighbours
            .iter()
            .find(|item| item.management_address.as_deref() == Some("192.168.60.12"))
            .unwrap();
        let hostname_only = dev
            .lldp
            .neighbours
            .iter()
            .find(|item| item.chassis_id.as_deref() == Some("DE:AD:BE:EF:00:11"))
            .unwrap();
        assert_eq!(resolved.resolution, "management-ip");
        assert_eq!(resolved.resolved_device_id, Some(3));
        assert_eq!(hostname_only.resolution, "unresolved");
        assert_eq!(hostname_only.resolved_device_id, None);
        let cdp_resolved = dev
            .cdp
            .neighbours
            .iter()
            .find(|item| item.management_address.as_deref() == Some("192.168.60.20"))
            .unwrap();
        let cdp_name = dev
            .cdp
            .neighbours
            .iter()
            .find(|item| item.sys_name.as_deref() == Some("AP-Lobby"))
            .unwrap();
        assert_eq!(cdp_resolved.resolution, "management-ip");
        assert_eq!(cdp_resolved.resolved_device_id, Some(4));
        assert_eq!(cdp_name.resolution, "unresolved");
        assert_eq!(cdp_name.resolved_device_id, None);
    }

    #[test]
    fn relationship_counts_include_rows_omitted_from_the_rendered_list() {
        let core = switch();
        let mut trace = CorrelationTrace::default();
        for index in 0..30u32 {
            trace.link(ObservedLink {
                source_ip: "192.168.60.2".into(),
                from_device_id: Some(2),
                to_device_id: if index < 5 { None } else { Some(4) },
                to_unresolved_id: if index < 5 {
                    Some(format!("unknown:{index}"))
                } else {
                    None
                },
                protocol: "fdb".into(),
                confidence: "strong".into(),
                from_port: Some(format!("Port {index}")),
                to_port: None,
                raw_port: index,
                port_role: "fdb".into(),
                chassis_or_mac: None,
                sys_name: None,
                management_address: None,
                resolution: if index < 5 { "unresolved" } else { "fdb-mac" }.into(),
                arp_ip: None,
            });
        }
        let snapshot = TopologySnapshot::empty("t");
        let diag = assemble(&[core], &targets(), &snapshot, &trace);
        let dev = device(&diag, "192.168.60.2");
        assert_eq!(dev.relationship_count, 30);
        assert_eq!(dev.relationships.len(), MAX_LISTED_RELATIONSHIPS);
        assert_eq!(dev.relationships_omitted, 30 - MAX_LISTED_RELATIONSHIPS);
        assert_eq!(dev.unresolved_peers, 5);
    }

    #[test]
    fn bridge_port_and_lldp_local_port_resolutions_are_explained() {
        let mut bridged = switch();
        bridged.interfaces.remove(&7);
        bridged.interfaces.insert(10107, iface(10107, "Gi1/0/7"));
        bridged.bridge_port_if.insert(7, 10107);
        bridged.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 7,
            chassis_id: Some("00:20:AA:00:00:01".into()),
            chassis_subtype: Some(4),
            port_id: Some("X0".into()),
            port_desc: None,
            sys_name: Some("FW1".into()),
            sys_desc: None,
            management_address: Some("192.168.60.1".into()),
        });
        let (_snap, diag) = correlate_detailed(&[bridged], &targets(), "t", None);
        let port = device(&diag, "192.168.60.2").relationships[0]
            .port
            .as_ref()
            .unwrap();
        assert_eq!(port.resolved_if_index, 10107);
        assert_eq!(port.display_label, "Gi1/0/7");
        assert!(
            port.resolution_source.contains("dot1dBasePortIfIndex"),
            "{}",
            port.resolution_source
        );

        let mut named = DeviceView::new(Ipv4Addr::new(192, 168, 60, 2), Some(2));
        named.sys_name = Some("NETGEAR-SW1".into());
        named.interfaces.insert(10107, iface(10107, "Gi1/0/7"));
        named.lldp_local_ports.insert(7, "Gi1/0/7".into());
        named.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 7,
            chassis_id: Some("00:20:AA:00:00:01".into()),
            chassis_subtype: Some(4),
            port_id: Some("X0".into()),
            port_desc: None,
            sys_name: Some("FW1".into()),
            sys_desc: None,
            management_address: Some("192.168.60.1".into()),
        });
        let (_snap, diag) = correlate_detailed(&[named], &targets(), "t", None);
        let port = device(&diag, "192.168.60.2").relationships[0]
            .port
            .as_ref()
            .unwrap();
        assert_eq!(port.display_label, "Gi1/0/7");
        assert!(
            port.resolution_source.contains("LLDP local-port table"),
            "{}",
            port.resolution_source
        );
    }

    #[test]
    fn malformed_port_label_falls_back_to_the_numeric_index() {
        let mut core = switch();
        core.interfaces.insert(
            24,
            Iface {
                index: 24,
                name: None,
                descr: None,
                alias: None,
                mac: None,
                if_type: Some(6),
                admin_status: Some(1),
                oper_status: Some(1),
                speed_mbps: None,
                poe: None,
            },
        );
        core.rejected_labels.push(RejectedLabel {
            field: "ifAlias".into(),
            if_index: 24,
        });
        let (_snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        let dev = device(&diag, "192.168.60.2");
        assert!(dev.suppressions.iter().any(|item| {
            item.reason == "malformed-port-label" && item.summary.contains("numeric index")
        }));
    }

    #[test]
    fn missing_lldp_with_usable_fdb_explains_the_strong_link() {
        let mut core = switch();
        core.probes.push(TableProbe {
            mib: "LLDP-MIB",
            table: "lldpRemChassisId",
            state: ProbeState::NoRows,
            rows: 0,
        });
        core.fdb.push(FdbEntry {
            mac: "00:11:32:00:00:20".into(),
            if_index: 12,
            vlan: Some(10),
        });
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        assert_eq!(snap.connections[0].protocol, "fdb");
        assert_ne!(
            snap.connections[0].confidence,
            TopologyConfidence::Confirmed
        );
        let dev = device(&diag, "192.168.60.2");
        assert!(dev
            .hints
            .iter()
            .any(|hint| hint.contains("LLDP may be disabled")));
        assert!(dev.zero_link_explanation.is_none());
    }

    #[test]
    fn only_if_mib_explains_why_there_are_no_links() {
        let mut core = DeviceView::new(Ipv4Addr::new(192, 168, 60, 2), Some(2));
        core.sys_name = Some("gateway".into());
        core.interfaces.insert(1, iface(1, "vlan1"));
        for (mib, table, state) in [
            ("IF-MIB", "ifDescr", ProbeState::Available),
            ("LLDP-MIB", "lldpRemChassisId", ProbeState::NoRows),
            ("CISCO-CDP-MIB", "cdpCacheDeviceId", ProbeState::NoRows),
            ("BRIDGE-MIB", "dot1dTpFdbPort", ProbeState::NoRows),
        ] {
            core.probes.push(TableProbe {
                mib,
                table,
                state,
                rows: if state == ProbeState::Available { 1 } else { 0 },
            });
        }
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        assert!(snap.connections.is_empty());
        let text = device(&diag, "192.168.60.2")
            .zero_link_explanation
            .clone()
            .unwrap();
        assert!(text.contains("IF-MIB responded"), "{text}");
    }

    #[test]
    fn vlan_access_trunk_and_poe_with_and_without_watts() {
        let mut core = switch();
        core.tagged.insert(24, BTreeSet::from([10, 20]));
        core.interfaces.get_mut(&12).unwrap().poe = Some(PoeInfo {
            enabled: true,
            watts: Some(8.2),
        });
        core.interfaces.get_mut(&7).unwrap().poe = Some(PoeInfo {
            enabled: true,
            watts: None,
        });
        core.probes.push(TableProbe {
            mib: "POWER-ETHERNET-MIB",
            table: "pethPsePortDetectionStatus",
            state: ProbeState::Available,
            rows: 2,
        });
        core.probes.push(TableProbe {
            mib: "CISCO-POWER-ETHERNET-EXT-MIB",
            table: "cpeExtPsePortPwrAllocated",
            state: ProbeState::NoRows,
            rows: 0,
        });
        let (_snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        let dev = device(&diag, "192.168.60.2");
        assert!(dev.vlan.access_ports >= 1);
        assert!(dev.vlan.trunk_ports >= 1);
        assert_eq!(dev.poe.enabled_ports, 2);
        assert_eq!(dev.poe.ports_with_watts, 1);
        assert_eq!(dev.poe.enabled_without_watts, 1);
    }

    #[test]
    fn entity_mib_partial_metadata_is_not_a_classification() {
        let mut core = switch();
        core.entity.model = Some("GS724".into());
        core.probes.push(TableProbe {
            mib: "ENTITY-MIB",
            table: "entPhysicalModelName",
            state: ProbeState::Available,
            rows: 3,
        });
        core.probes.push(TableProbe {
            mib: "ENTITY-MIB",
            table: "entPhysicalMfgName",
            state: ProbeState::WalkFailed,
            rows: 0,
        });
        let (_snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        let dev = device(&diag, "192.168.60.2");
        let entity = dev
            .mib_coverage
            .iter()
            .find(|item| item.mib == "ENTITY-MIB")
            .unwrap();
        assert_eq!(entity.state, MibState::Partial);
        assert!(dev
            .notes
            .iter()
            .any(|note| note.contains("GS724") && note.contains("not a device classification")));
    }

    #[test]
    fn timeout_is_isolated_from_a_responding_neighbour() {
        let mut core = switch();
        core.fdb.push(FdbEntry {
            mac: "00:11:32:00:00:20".into(),
            if_index: 12,
            vlan: Some(10),
        });
        let (snap, mut diag) = correlate_detailed(&[core], &targets(), "t", None);
        absorb_failures(
            &mut diag,
            &[TopologyDeviceFailure {
                ip: "192.168.60.5".into(),
                reason: "The device did not answer SNMP in time.".into(),
            }],
        );
        assert_eq!(snap.connections.len(), 1);
        let failed = device(&diag, "192.168.60.5");
        assert_eq!(failed.snmp_status, SnmpStatus::Timeout);
        assert!(failed
            .mib_coverage
            .iter()
            .all(|mib| mib.state == MibState::NotQueried));
        assert!(failed
            .zero_link_explanation
            .as_deref()
            .unwrap()
            .contains("timed out"));
        assert_eq!(
            device(&diag, "192.168.60.2").snmp_status,
            SnmpStatus::Responded
        );
    }

    #[test]
    fn gateway_identity_conflict_is_recorded_and_creates_no_edge() {
        let core = switch();
        let hint = EdgeHint {
            gateway_ip: Some("192.168.60.1".into()),
            gateway_mac: Some("00:1A:2B:00:00:02".into()),
        };
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", Some(&hint));
        assert!(snap.edge.is_none());
        assert!(diag
            .suppressions
            .iter()
            .any(|item| item.reason == "gateway-identity-conflict"));
    }

    #[test]
    fn diagnostics_do_not_change_an_ordinary_snapshot() {
        let mut core = switch();
        core.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 12,
            chassis_id: Some("00:1A:2B:00:00:12".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth0".into()),
            port_desc: None,
            sys_name: Some("AP-Lobby".into()),
            sys_desc: None,
            management_address: Some("192.168.60.12".into()),
        });
        core.fdb.push(FdbEntry {
            mac: "AA:BB:CC:00:00:50".into(),
            if_index: 7,
            vlan: Some(10),
        });
        let plain = correlate(&[core.clone()], &targets(), "t");
        let (detailed, diag) = correlate_detailed(&[core], &targets(), "t", None);
        assert_eq!(
            serde_json::to_value(&plain.connections).unwrap(),
            serde_json::to_value(&detailed.connections).unwrap()
        );
        assert!(!diag.devices.is_empty());
        let handoff = crate::serialize::preview_from_snapshot(&detailed, vec![], "id", "net", "t");
        let json = crate::serialize::handoff_preview_to_json(&handoff).unwrap();
        assert!(!json.contains("mibCoverage"));
        assert!(!json.contains("topology-diagnostics"));
    }

    #[test]
    fn large_fdb_diagnostics_stay_compact() {
        let mut core = switch();
        for port in 30..70u32 {
            for host in 1..6u8 {
                core.fdb.push(FdbEntry {
                    mac: format!("02:00:00:{port:02X}:00:{host:02X}"),
                    if_index: port,
                    vlan: Some(10),
                });
            }
        }
        let (snap, diag) = correlate_detailed(&[core], &targets(), "t", None);
        assert!(snap.connections.is_empty());
        let dev = device(&diag, "192.168.60.2");
        assert!(dev.fdb.unicast_macs > 100);
        assert!(dev.fdb.ports.len() <= MAX_PORT_DETAILS);
        assert!(dev.fdb.ports_omitted > 0);
        assert!(dev.suppressions.len() <= MAX_LISTED_SUPPRESSIONS);
        let rendered = serde_json::to_string(&dev.fdb).unwrap();
        assert!(
            rendered.len() < 8_000,
            "fdb diagnostic grew to {} bytes",
            rendered.len()
        );
        assert!(!rendered.contains("02:00:00:45:00:05"));
    }

    #[test]
    fn export_strips_credential_material() {
        let mut diag = TopologyDiagnostics::default();
        diag.correlation_notes.push(CorrelationNote {
            target_ip: "192.168.60.2".into(),
            summary: "community site-read-secret username monitor-user auth_password hunter2 priv_password priv-pass-xyz".into(),
        });
        let json = export_diagnostics_json(&diag);
        assert!(json.contains("network inventory information"));
        assert!(!json.contains("site-read-secret"));
        assert!(!json.contains("monitor-user"));
        assert!(!json.contains("hunter2"));
        assert!(!json.contains("priv-pass-xyz"));
        assert!(!json.contains("authPassword"));
    }

    struct SelectiveSession {
        inner: crate::snmp::FixtureSession,
        fail_prefix: String,
    }

    impl crate::snmp::SnmpSession for SelectiveSession {
        fn get<'a>(
            &'a self,
            oids: &'a [crate::ber::Oid],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<Vec<crate::ber::VarBind>, crate::error::TopologyError>,
                    > + Send
                    + 'a,
            >,
        > {
            self.inner.get(oids)
        }

        fn walk<'a>(
            &'a self,
            root: &'a crate::ber::Oid,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<Vec<crate::ber::VarBind>, crate::error::TopologyError>,
                    > + Send
                    + 'a,
            >,
        > {
            if root.to_dotted().starts_with(&self.fail_prefix) {
                return Box::pin(async { Err(crate::error::TopologyError::Timeout) });
            }
            self.inner.walk(root)
        }
    }

    #[tokio::test]
    async fn partial_mib_failure_does_not_drop_the_device() {
        use crate::ber::{Oid, SnmpValue};
        use crate::collect::{collect_device, IF_DESCR, SYS_NAME};
        use std::collections::BTreeMap;

        let mut values = BTreeMap::new();
        values.insert(
            Oid::from_slice(SYS_NAME).to_dotted(),
            SnmpValue::OctetString(b"core".to_vec()),
        );
        values.insert(
            format!("{}.1", Oid::from_slice(IF_DESCR)),
            SnmpValue::OctetString(b"Gi1/0/1".to_vec()),
        );
        let session = SelectiveSession {
            inner: crate::snmp::FixtureSession::new(values),
            fail_prefix: "1.0.8802".into(),
        };
        let view = collect_device(&session, Ipv4Addr::new(192, 168, 60, 7), Some(7))
            .await
            .unwrap();
        assert!(!view.interfaces.is_empty());
        let (_snap, diag) = correlate_detailed(&[view], &targets(), "t", None);
        let dev = device(&diag, "192.168.60.7");
        assert_eq!(dev.snmp_status, SnmpStatus::Responded);
        let lldp = dev
            .mib_coverage
            .iter()
            .find(|item| item.mib == "LLDP-MIB")
            .unwrap();
        let if_mib = dev
            .mib_coverage
            .iter()
            .find(|item| item.mib == "IF-MIB")
            .unwrap();
        assert_eq!(lldp.state, MibState::TimedOut);
        assert_eq!(if_mib.state, MibState::Available);
        assert!(dev.zero_link_explanation.is_some());
    }
}
