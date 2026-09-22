//! Versioned, credential-free topology evidence fixtures.
//!
//! The wire model is deliberately separate from [`DeviceView`]. Export is a
//! whitelist conversion from the parsed evidence the correlator consumes; raw
//! SNMP packets, sessions and credentials have no representable field here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::Ipv4Addr;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::collect::{
    ArpEntry, CdpNeighbor, DeviceView, EntityInfo, FdbEntry, Iface, LldpNeighbor, ProbeState,
    RejectedLabel, TableProbe,
};
use crate::credentials::{sanitize_text, SnmpSecret};
use crate::diagnostics::{absorb_failures, TopologyDiagnostics};
use crate::engine::TopologyRunCapture;
use crate::model::{
    EdgeHint, PoeInfo, TopologyConfidence, TopologyConnection, TopologyDeviceFailure,
    TopologyResult, TopologySummary, TopologyTarget,
};

pub const FIXTURE_VERSION: u32 = 1;
pub const MAX_REPLAY_FIXTURE_BYTES: usize = 8 * 1024 * 1024;
pub const REPLAY_WARNING: &str = "This fixture contains network inventory information (IP addresses, MAC addresses, hostnames, device and port names, and VLAN relationships). It never contains ArcScan credentials and is not uploaded automatically.";

#[derive(Debug)]
pub enum ReplayError {
    Json(String),
    UnsupportedVersion(u64),
    Invalid(String),
    TooLarge { bytes: usize, max: usize },
    Io(String),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(message) => write!(f, "Invalid topology replay fixture JSON: {message}"),
            Self::UnsupportedVersion(version) => write!(
                f,
                "Unsupported topology replay fixtureVersion {version}; this ArcScan build supports version {FIXTURE_VERSION}."
            ),
            Self::Invalid(message) => write!(f, "Invalid topology replay fixture: {message}"),
            Self::TooLarge { bytes, max } => write!(
                f,
                "Topology replay fixture is {bytes} bytes; the deterministic limit is {max} bytes."
            ),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ReplayError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayMetadata {
    pub description: String,
    pub sanitized: bool,
    pub warning: String,
}

impl Default for ReplayMetadata {
    fn default() -> Self {
        Self {
            description: "ArcScan topology evidence".into(),
            sanitized: false,
            warning: REPLAY_WARNING.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayFixture {
    pub fixture_version: u32,
    pub captured_at: String,
    pub source_version: String,
    pub targets: Vec<TopologyTarget>,
    pub device_views: Vec<ReplayDeviceView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_hint: Option<EdgeHint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<TopologyDeviceFailure>,
    pub metadata: ReplayMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<ReplayExpectations>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayExpectations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_links: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strong_links: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inferred_links: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_peers: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wan: Option<ExpectedWan>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub suppressed: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<ExpectedLink>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exact_links: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedLink {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_device_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_device_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_unresolved_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_port: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_port: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<TopologyConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mbps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_vlan: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tagged_vlans: Option<Vec<u16>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poe_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poe_watts: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExpectedWan {
    Present(bool),
    Facts(ExpectedWanFacts),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedWanFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_device_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_device_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_unresolved_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<TopologyConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uplink_protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uplink_from_port: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uplink_to_port: Option<String>,
}

impl ExpectedWan {
    fn present(&self) -> Option<bool> {
        match self {
            Self::Present(present) => Some(*present),
            Self::Facts(facts) => facts.present,
        }
    }

    fn facts(&self) -> Option<&ExpectedWanFacts> {
        match self {
            Self::Present(_) => None,
            Self::Facts(facts) => Some(facts),
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayDeviceView {
    pub target_ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_hint: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sys_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sys_descr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sys_object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chassis_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loc_sys_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<ReplayInterface>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lldp_local_ports: BTreeMap<u32, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lldp_neighbors: Vec<ReplayLldpNeighbor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cdp_neighbors: Vec<ReplayCdpNeighbor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fdb: Vec<ReplayFdbEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arp: Vec<ReplayArpEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pvid: BTreeMap<u32, u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tagged: BTreeMap<u32, BTreeSet<u16>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub untagged: BTreeMap<u32, BTreeSet<u16>>,
    #[serde(default)]
    pub entity: ReplayEntityInfo,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mibs_present: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bridge_port_if: BTreeMap<u32, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<ReplayTableProbe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_labels: Vec<ReplayRejectedLabel>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayInterface {
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_type: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_status: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oper_status: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mbps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poe: Option<PoeInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayLldpNeighbor {
    pub local_port_num: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chassis_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chassis_subtype: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sys_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sys_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub management_address: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayCdpNeighbor {
    pub if_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_port: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_vlan: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayFdbEntry {
    pub mac: String,
    pub if_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayArpEntry {
    pub ip: String,
    pub mac: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_index: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayEntityInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descr: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplayProbeState {
    Available,
    NoRows,
    TimedOut,
    WalkFailed,
    NotQueried,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayTableProbe {
    pub mib: String,
    pub table: String,
    pub state: ReplayProbeState,
    pub rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayRejectedLabel {
    pub field: String,
    pub if_index: u32,
}

impl ReplayFixture {
    pub fn from_capture(capture: &TopologyRunCapture, description: impl Into<String>) -> Self {
        let mut fixture = Self {
            fixture_version: FIXTURE_VERSION,
            captured_at: capture.captured_at.clone(),
            source_version: env!("CARGO_PKG_VERSION").into(),
            targets: capture.targets.clone(),
            device_views: capture.views.iter().map(ReplayDeviceView::from).collect(),
            edge_hint: capture.edge_hint.clone(),
            failures: capture.failures.clone(),
            metadata: ReplayMetadata {
                description: description.into(),
                ..ReplayMetadata::default()
            },
            expected: None,
        };
        fixture.normalize();
        fixture
    }

    pub fn parse(json: &str) -> Result<Self, ReplayError> {
        if json.len() > MAX_REPLAY_FIXTURE_BYTES {
            return Err(ReplayError::TooLarge {
                bytes: json.len(),
                max: MAX_REPLAY_FIXTURE_BYTES,
            });
        }
        let value: Value =
            serde_json::from_str(json).map_err(|e| ReplayError::Json(e.to_string()))?;
        let version = value
            .get("fixtureVersion")
            .and_then(Value::as_u64)
            .ok_or_else(|| ReplayError::Invalid("fixtureVersion must be an integer".into()))?;
        if version != u64::from(FIXTURE_VERSION) {
            return Err(ReplayError::UnsupportedVersion(version));
        }
        let mut fixture: Self =
            serde_json::from_value(value).map_err(|e| ReplayError::Json(e.to_string()))?;
        if !fixture.metadata.sanitized {
            return Err(ReplayError::Invalid(
                "metadata.sanitized must be true before a fixture can be replayed".into(),
            ));
        }
        fixture.normalize();
        Ok(fixture)
    }

    pub fn to_sanitized_json(&self, secret: Option<&SnmpSecret>) -> Result<String, ReplayError> {
        let mut value = serde_json::to_value(self).map_err(|e| ReplayError::Json(e.to_string()))?;
        scrub_value(&mut value, secret);
        let metadata = value
            .get_mut("metadata")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| ReplayError::Invalid("metadata must be an object".into()))?;
        // A captured fixture is intentionally unsanitized in memory. Only the
        // scrubbed export value may make this claim.
        metadata.insert("sanitized".into(), Value::Bool(true));
        let json =
            serde_json::to_string_pretty(&value).map_err(|e| ReplayError::Json(e.to_string()))?;
        if json.len() > MAX_REPLAY_FIXTURE_BYTES {
            return Err(ReplayError::TooLarge {
                bytes: json.len(),
                max: MAX_REPLAY_FIXTURE_BYTES,
            });
        }
        Ok(json)
    }

    pub fn replay(&self) -> Result<TopologyResult, ReplayError> {
        if self.fixture_version != FIXTURE_VERSION {
            return Err(ReplayError::UnsupportedVersion(u64::from(
                self.fixture_version,
            )));
        }
        let mut views: Vec<DeviceView> = self
            .device_views
            .iter()
            .map(ReplayDeviceView::to_device_view)
            .collect::<Result<_, _>>()?;
        views.sort_by_key(|view| view.target_ip);
        let mut targets = self.targets.clone();
        targets.sort_by(|a, b| (&a.ip, a.device_id).cmp(&(&b.ip, b.device_id)));
        let (mut snapshot, mut diagnostics) = crate::correlate::correlate_detailed(
            &views,
            &targets,
            &self.captured_at,
            self.edge_hint.as_ref(),
        );
        let mut failures = self.failures.clone();
        failures.sort_by(|a, b| (&a.ip, &a.reason).cmp(&(&b.ip, &b.reason)));
        absorb_failures(&mut diagnostics, &failures);
        normalize_snapshot(&mut snapshot);
        normalize_diagnostics(&mut diagnostics);
        diagnostics.run_summary.devices_queried = views.len() + failures.len();
        diagnostics.run_summary.devices_responding = views.len();
        diagnostics.run_summary.devices_failed = failures.len();
        let mut summary = summary_for(
            &snapshot.connections,
            snapshot.unknown_nodes.len(),
            &failures,
        );
        summary.devices_queried = views.len() + failures.len();
        summary.devices_responded = views.len();
        Ok(TopologyResult {
            snapshot,
            summary,
            diagnostics,
        })
    }

    fn normalize(&mut self) {
        self.targets
            .sort_by(|a, b| (&a.ip, a.device_id).cmp(&(&b.ip, b.device_id)));
        self.device_views
            .sort_by(|a, b| a.target_ip.cmp(&b.target_ip));
        self.failures
            .sort_by(|a, b| (&a.ip, &a.reason).cmp(&(&b.ip, &b.reason)));
        if let Some(expected) = &mut self.expected {
            for link in &mut expected.links {
                if let Some(tagged_vlans) = &mut link.tagged_vlans {
                    tagged_vlans.sort_unstable();
                    tagged_vlans.dedup();
                }
            }
        }
        for view in &mut self.device_views {
            view.normalize();
        }
    }
}

impl ReplayDeviceView {
    fn normalize(&mut self) {
        self.interfaces.sort_by_key(|iface| iface.index);
        self.lldp_neighbors.sort_by(|a, b| {
            (a.local_port_num, &a.chassis_id, &a.port_id).cmp(&(
                b.local_port_num,
                &b.chassis_id,
                &b.port_id,
            ))
        });
        self.cdp_neighbors.sort_by(|a, b| {
            (a.if_index, &a.device_id, &a.device_port).cmp(&(
                b.if_index,
                &b.device_id,
                &b.device_port,
            ))
        });
        self.fdb
            .sort_by(|a, b| (a.if_index, &a.mac, a.vlan).cmp(&(b.if_index, &b.mac, b.vlan)));
        self.arp
            .sort_by(|a, b| (&a.ip, &a.mac, a.if_index).cmp(&(&b.ip, &b.mac, b.if_index)));
        self.mibs_present.sort();
        self.mibs_present.dedup();
        self.notes.sort();
        self.probes
            .sort_by(|a, b| (&a.mib, &a.table).cmp(&(&b.mib, &b.table)));
        self.rejected_labels
            .sort_by(|a, b| (&a.field, a.if_index).cmp(&(&b.field, b.if_index)));
    }

    fn to_device_view(&self) -> Result<DeviceView, ReplayError> {
        let ip: Ipv4Addr = self.target_ip.parse().map_err(|_| {
            ReplayError::Invalid(format!("{} is not a valid IPv4 target", self.target_ip))
        })?;
        let interfaces = self
            .interfaces
            .iter()
            .map(|iface| {
                (
                    iface.index,
                    Iface {
                        index: iface.index,
                        descr: iface.descr.clone(),
                        name: iface.name.clone(),
                        alias: iface.alias.clone(),
                        mac: iface.mac.clone(),
                        if_type: iface.if_type,
                        admin_status: iface.admin_status,
                        oper_status: iface.oper_status,
                        speed_mbps: iface.speed_mbps,
                        poe: iface.poe.clone(),
                    },
                )
            })
            .collect();
        let probes = self
            .probes
            .iter()
            .map(|probe| {
                Ok(TableProbe {
                    mib: known_mib(&probe.mib)?,
                    table: known_table(&probe.table)?,
                    state: probe.state.into(),
                    rows: probe.rows,
                })
            })
            .collect::<Result<_, ReplayError>>()?;
        Ok(DeviceView {
            target_ip: ip,
            inventory_hint: self.inventory_hint,
            sys_name: self.sys_name.clone(),
            sys_descr: self.sys_descr.clone(),
            sys_object_id: self.sys_object_id.clone(),
            bridge_address: self.bridge_address.clone(),
            chassis_id: self.chassis_id.clone(),
            loc_sys_name: self.loc_sys_name.clone(),
            interfaces,
            lldp_local_ports: self.lldp_local_ports.clone(),
            lldp_neighbors: self
                .lldp_neighbors
                .iter()
                .cloned()
                .map(Into::into)
                .collect(),
            cdp_neighbors: self.cdp_neighbors.iter().cloned().map(Into::into).collect(),
            fdb: self.fdb.iter().cloned().map(Into::into).collect(),
            arp: self.arp.iter().cloned().map(Into::into).collect(),
            pvid: self.pvid.clone(),
            tagged: self.tagged.clone(),
            untagged: self.untagged.clone(),
            entity: EntityInfo {
                manufacturer: self.entity.manufacturer.clone(),
                model: self.entity.model.clone(),
                descr: self.entity.descr.clone(),
            },
            mibs_present: self.mibs_present.clone(),
            notes: self.notes.clone(),
            bridge_port_if: self.bridge_port_if.clone(),
            probes,
            rejected_labels: self
                .rejected_labels
                .iter()
                .map(|label| RejectedLabel {
                    field: label.field.clone(),
                    if_index: label.if_index,
                })
                .collect(),
        })
    }
}

impl From<&DeviceView> for ReplayDeviceView {
    fn from(view: &DeviceView) -> Self {
        Self {
            target_ip: view.target_ip.to_string(),
            inventory_hint: view.inventory_hint,
            sys_name: view.sys_name.clone(),
            sys_descr: view.sys_descr.clone(),
            sys_object_id: view.sys_object_id.clone(),
            bridge_address: view.bridge_address.clone(),
            chassis_id: view.chassis_id.clone(),
            loc_sys_name: view.loc_sys_name.clone(),
            interfaces: view
                .interfaces
                .values()
                .map(ReplayInterface::from)
                .collect(),
            lldp_local_ports: view.lldp_local_ports.clone(),
            lldp_neighbors: view
                .lldp_neighbors
                .iter()
                .map(ReplayLldpNeighbor::from)
                .collect(),
            cdp_neighbors: view
                .cdp_neighbors
                .iter()
                .map(ReplayCdpNeighbor::from)
                .collect(),
            fdb: view.fdb.iter().map(ReplayFdbEntry::from).collect(),
            arp: view.arp.iter().map(ReplayArpEntry::from).collect(),
            pvid: view.pvid.clone(),
            tagged: view.tagged.clone(),
            untagged: view.untagged.clone(),
            entity: ReplayEntityInfo {
                manufacturer: view.entity.manufacturer.clone(),
                model: view.entity.model.clone(),
                descr: view.entity.descr.clone(),
            },
            mibs_present: view.mibs_present.clone(),
            notes: view.notes.clone(),
            bridge_port_if: view.bridge_port_if.clone(),
            probes: view.probes.iter().map(ReplayTableProbe::from).collect(),
            rejected_labels: view
                .rejected_labels
                .iter()
                .map(|label| ReplayRejectedLabel {
                    field: label.field.clone(),
                    if_index: label.if_index,
                })
                .collect(),
        }
    }
}

macro_rules! copy_from {
    ($source:ty => $target:ty { $($field:ident),+ $(,)? }) => {
        impl From<&$source> for $target {
            fn from(value: &$source) -> Self {
                Self { $($field: value.$field.clone()),+ }
            }
        }
    };
}

copy_from!(Iface => ReplayInterface { index, descr, name, alias, mac, if_type, admin_status, oper_status, speed_mbps, poe });
copy_from!(LldpNeighbor => ReplayLldpNeighbor { local_port_num, chassis_id, chassis_subtype, port_id, port_desc, sys_name, sys_desc, management_address });
copy_from!(CdpNeighbor => ReplayCdpNeighbor { if_index, device_id, device_port, platform, address, native_vlan });
copy_from!(FdbEntry => ReplayFdbEntry { mac, if_index, vlan });
copy_from!(ArpEntry => ReplayArpEntry { ip, mac, if_index });

impl From<ReplayLldpNeighbor> for LldpNeighbor {
    fn from(value: ReplayLldpNeighbor) -> Self {
        Self {
            local_port_num: value.local_port_num,
            chassis_id: value.chassis_id,
            chassis_subtype: value.chassis_subtype,
            port_id: value.port_id,
            port_desc: value.port_desc,
            sys_name: value.sys_name,
            sys_desc: value.sys_desc,
            management_address: value.management_address,
        }
    }
}

impl From<ReplayCdpNeighbor> for CdpNeighbor {
    fn from(value: ReplayCdpNeighbor) -> Self {
        Self {
            if_index: value.if_index,
            device_id: value.device_id,
            device_port: value.device_port,
            platform: value.platform,
            address: value.address,
            native_vlan: value.native_vlan,
        }
    }
}

impl From<ReplayFdbEntry> for FdbEntry {
    fn from(value: ReplayFdbEntry) -> Self {
        Self {
            mac: value.mac,
            if_index: value.if_index,
            vlan: value.vlan,
        }
    }
}

impl From<ReplayArpEntry> for ArpEntry {
    fn from(value: ReplayArpEntry) -> Self {
        Self {
            ip: value.ip,
            mac: value.mac,
            if_index: value.if_index,
        }
    }
}

impl From<ProbeState> for ReplayProbeState {
    fn from(value: ProbeState) -> Self {
        match value {
            ProbeState::Available => Self::Available,
            ProbeState::NoRows => Self::NoRows,
            ProbeState::TimedOut => Self::TimedOut,
            ProbeState::WalkFailed => Self::WalkFailed,
            ProbeState::NotQueried => Self::NotQueried,
        }
    }
}

impl From<ReplayProbeState> for ProbeState {
    fn from(value: ReplayProbeState) -> Self {
        match value {
            ReplayProbeState::Available => Self::Available,
            ReplayProbeState::NoRows => Self::NoRows,
            ReplayProbeState::TimedOut => Self::TimedOut,
            ReplayProbeState::WalkFailed => Self::WalkFailed,
            ReplayProbeState::NotQueried => Self::NotQueried,
        }
    }
}

impl From<&TableProbe> for ReplayTableProbe {
    fn from(value: &TableProbe) -> Self {
        Self {
            mib: value.mib.into(),
            table: value.table.into(),
            state: value.state.into(),
            rows: value.rows,
        }
    }
}

fn known_mib(value: &str) -> Result<&'static str, ReplayError> {
    match value {
        "IF-MIB" => Ok("IF-MIB"),
        "BRIDGE-MIB" => Ok("BRIDGE-MIB"),
        "Q-BRIDGE-MIB" => Ok("Q-BRIDGE-MIB"),
        "IP-MIB" => Ok("IP-MIB"),
        "LLDP-MIB" => Ok("LLDP-MIB"),
        "CISCO-CDP-MIB" => Ok("CISCO-CDP-MIB"),
        "POWER-ETHERNET-MIB" => Ok("POWER-ETHERNET-MIB"),
        "CISCO-POWER-ETHERNET-EXT-MIB" => Ok("CISCO-POWER-ETHERNET-EXT-MIB"),
        "ENTITY-MIB" => Ok("ENTITY-MIB"),
        _ => Err(ReplayError::Invalid(format!("unknown MIB name {value:?}"))),
    }
}

fn known_table(value: &str) -> Result<&'static str, ReplayError> {
    match value {
        "ifDescr" => Ok("ifDescr"),
        "ifName" => Ok("ifName"),
        "ifAlias" => Ok("ifAlias"),
        "ifType" => Ok("ifType"),
        "ifPhysAddress" => Ok("ifPhysAddress"),
        "ifOperStatus" => Ok("ifOperStatus"),
        "ifAdminStatus" => Ok("ifAdminStatus"),
        "ifHighSpeed" => Ok("ifHighSpeed"),
        "ifSpeed" => Ok("ifSpeed"),
        "dot1dBasePortIfIndex" => Ok("dot1dBasePortIfIndex"),
        "dot1dTpFdbPort" => Ok("dot1dTpFdbPort"),
        "dot1dTpFdbStatus" => Ok("dot1dTpFdbStatus"),
        "dot1qTpFdbPort" => Ok("dot1qTpFdbPort"),
        "dot1qPvid" => Ok("dot1qPvid"),
        "dot1qVlanCurrentEgressPorts" => Ok("dot1qVlanCurrentEgressPorts"),
        "dot1qVlanCurrentUntaggedPorts" => Ok("dot1qVlanCurrentUntaggedPorts"),
        "ipNetToMediaPhysAddress" => Ok("ipNetToMediaPhysAddress"),
        "ipNetToPhysicalPhysAddress" => Ok("ipNetToPhysicalPhysAddress"),
        "lldpLocPortId" => Ok("lldpLocPortId"),
        "lldpLocPortDesc" => Ok("lldpLocPortDesc"),
        "lldpRemChassisId" => Ok("lldpRemChassisId"),
        "lldpRemChassisIdSubtype" => Ok("lldpRemChassisIdSubtype"),
        "lldpRemPortId" => Ok("lldpRemPortId"),
        "lldpRemPortDesc" => Ok("lldpRemPortDesc"),
        "lldpRemSysName" => Ok("lldpRemSysName"),
        "lldpRemSysDesc" => Ok("lldpRemSysDesc"),
        "lldpRemManAddr" => Ok("lldpRemManAddr"),
        "cdpCacheDeviceId" => Ok("cdpCacheDeviceId"),
        "cdpCacheDevicePort" => Ok("cdpCacheDevicePort"),
        "cdpCachePlatform" => Ok("cdpCachePlatform"),
        "cdpCacheAddress" => Ok("cdpCacheAddress"),
        "cdpCacheNativeVLAN" => Ok("cdpCacheNativeVLAN"),
        "pethPsePortDetectionStatus" => Ok("pethPsePortDetectionStatus"),
        "cpeExtPsePortPwrAllocated" => Ok("cpeExtPsePortPwrAllocated"),
        "entPhysicalModelName" => Ok("entPhysicalModelName"),
        "entPhysicalMfgName" => Ok("entPhysicalMfgName"),
        "entPhysicalDescr" => Ok("entPhysicalDescr"),
        _ => Err(ReplayError::Invalid(format!(
            "unknown table name {value:?}"
        ))),
    }
}

fn summary_for(
    connections: &[TopologyConnection],
    unknown_nodes: usize,
    failures: &[TopologyDeviceFailure],
) -> TopologySummary {
    let (mut confirmed, mut strong, mut inferred) = (0, 0, 0);
    for connection in connections {
        match connection.confidence {
            TopologyConfidence::Confirmed => confirmed += 1,
            TopologyConfidence::Strong => strong += 1,
            TopologyConfidence::Inferred => inferred += 1,
        }
    }
    TopologySummary {
        devices_queried: failures.len(),
        devices_responded: 0,
        devices_failed: failures.len(),
        confirmed,
        strong,
        inferred,
        unknown_nodes,
        duration_ms: 0,
        cancelled: false,
        timed_out: false,
        failures: failures.to_vec(),
    }
}

fn normalize_snapshot(snapshot: &mut crate::model::TopologySnapshot) {
    for connection in &mut snapshot.connections {
        connection.evidence.sort();
        connection.tagged_vlans.sort_unstable();
        connection.tagged_vlans.dedup();
    }
    snapshot
        .connections
        .sort_by(|a, b| connection_key(a).cmp(&connection_key(b)));
    snapshot.unknown_nodes.sort_by(|a, b| a.id.cmp(&b.id));
    snapshot.logical_nodes.sort_by(|a, b| a.id.cmp(&b.id));
    if let Some(edge) = &mut snapshot.edge {
        edge.evidence.sort();
        edge.uplink.evidence.sort();
        edge.uplink.tagged_vlans.sort_unstable();
        edge.uplink.tagged_vlans.dedup();
    }
}

fn normalize_diagnostics(diagnostics: &mut TopologyDiagnostics) {
    diagnostics
        .devices
        .sort_by(|a, b| a.target_ip.cmp(&b.target_ip));
    diagnostics.suppressions.sort_by(|a, b| {
        (&a.target_ip, &a.reason, &a.port_label, &a.summary).cmp(&(
            &b.target_ip,
            &b.reason,
            &b.port_label,
            &b.summary,
        ))
    });
    diagnostics
        .correlation_notes
        .sort_by(|a, b| (&a.target_ip, &a.summary).cmp(&(&b.target_ip, &b.summary)));
    for device in &mut diagnostics.devices {
        device.relationships.sort_by(|a, b| {
            (
                a.from_device_id,
                a.to_device_id,
                &a.to_unresolved_id,
                &a.from_port,
                &a.protocol,
            )
                .cmp(&(
                    b.from_device_id,
                    b.to_device_id,
                    &b.to_unresolved_id,
                    &b.from_port,
                    &b.protocol,
                ))
        });
        device.suppressions.sort_by(|a, b| {
            (&a.reason, &a.port_label, &a.summary).cmp(&(&b.reason, &b.port_label, &b.summary))
        });
        device.port_mappings.sort_by(|a, b| {
            (&a.role, a.raw_port, a.resolved_if_index).cmp(&(
                &b.role,
                b.raw_port,
                b.resolved_if_index,
            ))
        });
        device.notes.sort();
        device.hints.sort();
    }
}

type ConnectionSortKey<'a> = (
    Option<i64>,
    Option<i64>,
    &'a Option<String>,
    &'a Option<String>,
    &'a str,
    &'a str,
);

fn connection_key(connection: &TopologyConnection) -> ConnectionSortKey<'_> {
    (
        connection.from_device_id,
        connection.to_device_id,
        &connection.to_unresolved_id,
        &connection.from_port,
        &connection.protocol,
        connection.confidence.as_str(),
    )
}

fn scrub_value(value: &mut Value, secret: Option<&SnmpSecret>) {
    match value {
        Value::String(text) => *text = scrub_text(text, secret),
        Value::Array(items) => {
            for item in items {
                scrub_value(item, secret);
            }
        }
        Value::Object(map) => {
            map.retain(|key, _| !is_forbidden_key(key));
            for child in map.values_mut() {
                scrub_value(child, secret);
            }
        }
        _ => {}
    }
}

fn is_forbidden_key(key: &str) -> bool {
    let compact: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        compact.as_str(),
        "community"
            | "username"
            | "authpassword"
            | "privpassword"
            | "windowspassword"
            | "arcatlastoken"
            | "apikey"
            | "sessioncredentialstate"
            | "environment"
            | "env"
    )
}

fn scrub_text(text: &str, secret: Option<&SnmpSecret>) -> String {
    let mut out = sanitize_text(text.to_string(), secret);
    for needle in [
        "username=",
        "username ",
        "password=",
        "password ",
        "windows_password=",
        "windows password ",
        "api_key=",
        "apikey=",
        "api key ",
        "token=",
        "token ",
        "bearer ",
        "authorization:",
    ] {
        out = redact_after(&out, needle);
    }
    out
}

fn redact_after(text: &str, needle: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    let lower_needle = needle.to_ascii_lowercase();
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(index) = lower.find(&lower_needle) else {
            out.push_str(rest);
            return out;
        };
        let start = index + needle.len();
        out.push_str(&rest[..start]);
        out.push_str("[redacted]");
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '"' | '\''))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
}

pub fn replay_fixture(json: &str) -> Result<TopologyResult, ReplayError> {
    ReplayFixture::parse(json)?.replay()
}

pub fn compare_expected(fixture: &ReplayFixture, result: &TopologyResult) -> String {
    let Some(expected) = fixture.expected.as_ref() else {
        return String::new();
    };
    let mut changes = Vec::new();
    compare_count(
        &mut changes,
        "confirmed links",
        expected.confirmed_links,
        result.diagnostics.run_summary.confirmed_links,
    );
    compare_count(
        &mut changes,
        "strong links",
        expected.strong_links,
        result.diagnostics.run_summary.strong_links,
    );
    compare_count(
        &mut changes,
        "inferred links",
        expected.inferred_links,
        result.diagnostics.run_summary.inferred_links,
    );
    compare_count(
        &mut changes,
        "unresolved peers",
        expected.unresolved_peers,
        result.snapshot.unknown_nodes.len(),
    );
    if let Some(wan) = expected.wan.as_ref() {
        let edge = result.snapshot.edge.as_ref();
        let mut fields = Vec::new();
        if wan
            .present()
            .is_some_and(|present| present != edge.is_some())
        {
            fields.push(format!(
                "present:\nexpected {:?}\nactual {}",
                wan.present(),
                edge.is_some()
            ));
        }
        if let Some(wan) = wan.facts() {
            compare_optional_field(
                &mut fields,
                "gatewayDeviceId",
                wan.gateway_device_id.as_ref(),
                edge.and_then(|edge| edge.gateway_device_id.as_ref()),
            );
            compare_optional_field(
                &mut fields,
                "gatewayIp",
                wan.gateway_ip.as_ref(),
                edge.and_then(|edge| edge.gateway_ip.as_ref()),
            );
            compare_optional_field(
                &mut fields,
                "gatewayMac",
                wan.gateway_mac.as_ref(),
                edge.and_then(|edge| edge.gateway_mac.as_ref()),
            );
            compare_optional_field(
                &mut fields,
                "viaDeviceId",
                wan.via_device_id.as_ref(),
                edge.and_then(|edge| edge.via_device_id.as_ref()),
            );
            compare_optional_field(
                &mut fields,
                "viaUnresolvedId",
                wan.via_unresolved_id.as_ref(),
                edge.and_then(|edge| edge.via_unresolved_id.as_ref()),
            );
            compare_optional_field(
                &mut fields,
                "confidence",
                wan.confidence.as_ref(),
                edge.map(|edge| &edge.confidence),
            );
            compare_optional_field(
                &mut fields,
                "uplink.protocol",
                wan.uplink_protocol.as_ref(),
                edge.map(|edge| &edge.uplink.protocol),
            );
            compare_optional_field(
                &mut fields,
                "uplink.fromPort",
                wan.uplink_from_port.as_ref(),
                edge.and_then(|edge| edge.uplink.from_port.as_ref()),
            );
            compare_optional_field(
                &mut fields,
                "uplink.toPort",
                wan.uplink_to_port.as_ref(),
                edge.and_then(|edge| edge.uplink.to_port.as_ref()),
            );
        }
        if !fields.is_empty() {
            changes.push(format!("CHANGED\nWAN\n{}", fields.join("\n")));
        }
    }
    for (reason, wanted) in &expected.suppressed {
        let actual = result
            .diagnostics
            .suppressions
            .iter()
            .filter(|item| item.reason == *reason)
            .count();
        if actual != *wanted {
            changes.push(format!(
                "CHANGED\nsuppressed {reason}\nexpected: {wanted}\nactual: {actual}"
            ));
        }
    }
    for wanted in &expected.links {
        if let Some(actual) = result
            .snapshot
            .connections
            .iter()
            .find(|link| same_endpoints(wanted, link))
        {
            let fields = link_field_changes(wanted, actual);
            if !fields.is_empty() {
                changes.push(format!(
                    "CHANGED\n{}\n{}",
                    expected_link_label(wanted, &fixture.targets),
                    fields.join("\n")
                ));
            }
        } else {
            let related = related_diagnostics(wanted, result);
            changes.push(format!(
                "REMOVED\n{}{}",
                expected_link_label(wanted, &fixture.targets),
                related
            ));
        }
    }
    if expected.exact_links {
        for actual in &result.snapshot.connections {
            if !expected
                .links
                .iter()
                .any(|wanted| same_link(wanted, actual))
            {
                changes.push(format!(
                    "ADDED\n{}",
                    actual_link_label(actual, &fixture.targets)
                ));
            }
        }
    }
    changes.join("\n\n")
}

fn compare_count(changes: &mut Vec<String>, label: &str, expected: Option<usize>, actual: usize) {
    if let Some(expected) = expected {
        if expected != actual {
            changes.push(format!(
                "CHANGED\n{label}\nexpected: {expected}\nactual: {actual}"
            ));
        }
    }
}

fn compare_optional_field<T: PartialEq + fmt::Debug>(
    fields: &mut Vec<String>,
    label: &str,
    expected: Option<&T>,
    actual: Option<&T>,
) {
    if let Some(expected) = expected {
        if Some(expected) != actual {
            fields.push(format!(
                "{label}:\nexpected {expected:?}\nactual {actual:?}"
            ));
        }
    }
}

fn link_field_changes(expected: &ExpectedLink, actual: &TopologyConnection) -> Vec<String> {
    let mut fields = Vec::new();
    compare_optional_field(
        &mut fields,
        "toPort",
        expected.to_port.as_ref(),
        actual.to_port.as_ref(),
    );
    compare_optional_field(
        &mut fields,
        "protocol",
        expected.protocol.as_ref(),
        Some(&actual.protocol),
    );
    compare_optional_field(
        &mut fields,
        "confidence",
        expected.confidence.as_ref(),
        Some(&actual.confidence),
    );
    compare_optional_field(
        &mut fields,
        "kind",
        expected.kind.as_ref(),
        Some(&actual.kind),
    );
    compare_optional_field(
        &mut fields,
        "speedMbps",
        expected.speed_mbps.as_ref(),
        actual.speed_mbps.as_ref(),
    );
    compare_optional_field(
        &mut fields,
        "vlan",
        expected.vlan.as_ref(),
        actual.vlan.as_ref(),
    );
    compare_optional_field(
        &mut fields,
        "nativeVlan",
        expected.native_vlan.as_ref(),
        actual.native_vlan.as_ref(),
    );
    compare_optional_field(
        &mut fields,
        "taggedVlans",
        expected.tagged_vlans.as_ref(),
        Some(&actual.tagged_vlans),
    );
    compare_optional_field(
        &mut fields,
        "poe.enabled",
        expected.poe_enabled.as_ref(),
        actual.poe.as_ref().map(|poe| &poe.enabled),
    );
    compare_optional_field(
        &mut fields,
        "poe.watts",
        expected.poe_watts.as_ref(),
        actual.poe.as_ref().and_then(|poe| poe.watts.as_ref()),
    );
    fields
}

fn same_endpoints(expected: &ExpectedLink, actual: &TopologyConnection) -> bool {
    expected.from_device_id == actual.from_device_id
        && expected.to_device_id == actual.to_device_id
        && expected.to_unresolved_id == actual.to_unresolved_id
        && expected.from_port == actual.from_port
}

fn same_link(expected: &ExpectedLink, actual: &TopologyConnection) -> bool {
    same_endpoints(expected, actual) && link_field_changes(expected, actual).is_empty()
}

fn expected_link_label(expected: &ExpectedLink, targets: &[TopologyTarget]) -> String {
    let from = endpoint_label(expected.from_device_id, None, targets);
    let to = endpoint_label(
        expected.to_device_id,
        expected.to_unresolved_id.as_deref(),
        targets,
    );
    match expected.from_port.as_deref() {
        Some(port) => format!("{from} {port} -> {to}"),
        None => format!("{from} -> {to}"),
    }
}

fn actual_link_label(actual: &TopologyConnection, targets: &[TopologyTarget]) -> String {
    let from = endpoint_label(
        actual.from_device_id,
        actual.from_unresolved_id.as_deref(),
        targets,
    );
    let to = endpoint_label(
        actual.to_device_id,
        actual.to_unresolved_id.as_deref(),
        targets,
    );
    let port = actual
        .from_port
        .as_deref()
        .map(|port| format!(" {port}"))
        .unwrap_or_default();
    format!(
        "{from}{port} -> {to}\nprotocol: {}\nconfidence: {}",
        actual.protocol,
        actual.confidence.as_str()
    )
}

fn endpoint_label(id: Option<i64>, unresolved: Option<&str>, targets: &[TopologyTarget]) -> String {
    if let Some(id) = id {
        if let Some(target) = targets.iter().find(|target| target.device_id == Some(id)) {
            return target
                .detected_name
                .clone()
                .or_else(|| target.hostname.clone())
                .unwrap_or_else(|| target.ip.clone());
        }
        return format!("device:{id}");
    }
    unresolved.unwrap_or("unknown").to_string()
}

fn related_diagnostics(expected: &ExpectedLink, result: &TopologyResult) -> String {
    let related: Vec<String> = result
        .diagnostics
        .suppressions
        .iter()
        .filter(|item| {
            item.inventory_device_id == expected.from_device_id
                && (expected.from_port.is_none() || item.port_label == expected.from_port)
        })
        .map(|item| item.summary.clone())
        .collect();
    if related.is_empty() {
        String::new()
    } else {
        format!("\n\nRelated diagnostics:\n{}", related.join("\n"))
    }
}

pub fn assert_topology_fixture(path: impl AsRef<Path>) {
    let path = path.as_ref();
    let json = std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "Could not read topology fixture {}: {error}",
            path.display()
        )
    });
    let fixture = ReplayFixture::parse(&json).unwrap_or_else(|error| {
        panic!(
            "Could not parse topology fixture {}: {error}",
            path.display()
        )
    });
    let result = fixture.replay().unwrap_or_else(|error| {
        panic!(
            "Could not replay topology fixture {}: {error}",
            path.display()
        )
    });
    let diff = compare_expected(&fixture, &result);
    assert!(
        diff.is_empty(),
        "Topology fixture {} did not match:\n\n{diff}",
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::CredentialInput;

    fn rich_capture() -> TopologyRunCapture {
        let targets = vec![
            TopologyTarget {
                ip: "192.0.2.1".into(),
                mac: Some("00:11:22:00:00:01".into()),
                device_id: Some(1),
                hostname: Some("FIREWALL".into()),
                detected_name: Some("Firewall".into()),
            },
            TopologyTarget {
                ip: "192.0.2.9".into(),
                mac: Some("00:11:22:00:00:09".into()),
                device_id: Some(9),
                hostname: Some("FIBER-ONT".into()),
                detected_name: Some("Fiber ONT".into()),
            },
        ];
        let mut view = DeviceView::new(Ipv4Addr::new(192, 0, 2, 1), Some(1));
        view.sys_name = Some("FIREWALL".into());
        view.interfaces.insert(
            9,
            Iface {
                index: 9,
                name: Some("WAN".into()),
                oper_status: Some(1),
                speed_mbps: Some(1000),
                poe: Some(PoeInfo {
                    enabled: true,
                    watts: Some(8.2),
                }),
                ..Iface::default()
            },
        );
        view.pvid.insert(9, 10);
        view.tagged.insert(9, BTreeSet::from([20, 30]));
        view.lldp_neighbors.push(LldpNeighbor {
            local_port_num: 9,
            chassis_id: Some("00:11:22:00:00:09".into()),
            chassis_subtype: Some(4),
            port_id: Some("eth0".into()),
            port_desc: None,
            sys_name: Some("Fiber ONT".into()),
            sys_desc: None,
            management_address: Some("192.0.2.9".into()),
        });
        TopologyRunCapture {
            captured_at: "2026-09-21T12:00:00Z".into(),
            targets,
            views: vec![view],
            edge_hint: Some(EdgeHint {
                gateway_ip: Some("192.0.2.1".into()),
                gateway_mac: Some("00:11:22:00:00:01".into()),
            }),
            failures: vec![],
        }
    }

    fn malicious_fixture() -> ReplayFixture {
        ReplayFixture {
            fixture_version: FIXTURE_VERSION,
            captured_at: "2026-09-21T12:00:00Z".into(),
            source_version: "test".into(),
            targets: vec![TopologyTarget {
                ip: "192.0.2.1".into(),
                mac: None,
                device_id: Some(1),
                hostname: Some("username=snmpadmin".into()),
                detected_name: Some("ArcAtlas bearer token atlas-secret".into()),
            }],
            device_views: vec![ReplayDeviceView {
                target_ip: "192.0.2.1".into(),
                inventory_hint: Some(1),
                sys_name: Some("community=super-secret-community".into()),
                sys_descr: Some("auth_password=hunter2 priv_password=secret-privacy".into()),
                sys_object_id: None,
                bridge_address: None,
                chassis_id: None,
                loc_sys_name: None,
                interfaces: vec![],
                lldp_local_ports: BTreeMap::new(),
                lldp_neighbors: vec![],
                cdp_neighbors: vec![],
                fdb: vec![],
                arp: vec![],
                pvid: BTreeMap::new(),
                tagged: BTreeMap::new(),
                untagged: BTreeMap::new(),
                entity: ReplayEntityInfo {
                    manufacturer: Some("apiKey=key-secret".into()),
                    model: Some("Windows password windows-secret".into()),
                    descr: None,
                },
                mibs_present: vec![],
                notes: vec!["Authorization: Bearer atlas-secret".into()],
                bridge_port_if: BTreeMap::new(),
                probes: vec![],
                rejected_labels: vec![],
            }],
            edge_hint: None,
            failures: vec![TopologyDeviceFailure {
                ip: "192.0.2.2".into(),
                reason: "password=hunter2 token=atlas-secret".into(),
            }],
            metadata: ReplayMetadata {
                description: "nested token atlas-secret".into(),
                ..ReplayMetadata::default()
            },
            expected: None,
        }
    }

    #[test]
    fn export_is_structurally_credential_free_and_scrubs_embedded_secrets() {
        let fixture = malicious_fixture();
        assert!(!fixture.metadata.sanitized);
        let secret = SnmpSecret::from_input(CredentialInput {
            version: "v3".into(),
            community: None,
            username: Some("snmpadmin".into()),
            auth_protocol: Some("sha256".into()),
            auth_password: Some("hunter2".into()),
            priv_protocol: Some("aes128".into()),
            priv_password: Some("secret-privacy".into()),
            context: None,
        })
        .unwrap();
        let json = fixture.to_sanitized_json(Some(&secret)).unwrap();
        for forbidden in [
            "super-secret-community",
            "snmpadmin",
            "hunter2",
            "secret-privacy",
            "atlas-secret",
            "key-secret",
            "windows-secret",
        ] {
            assert!(!json.contains(forbidden), "leaked {forbidden}: {json}");
        }
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value.pointer("/metadata/sanitized"),
            Some(&Value::Bool(true))
        );
        fn keys(value: &Value, out: &mut Vec<String>) {
            match value {
                Value::Object(map) => {
                    for (key, child) in map {
                        out.push(key.clone());
                        keys(child, out);
                    }
                }
                Value::Array(items) => items.iter().for_each(|item| keys(item, out)),
                _ => {}
            }
        }
        let mut all_keys = Vec::new();
        keys(&value, &mut all_keys);
        assert!(all_keys.iter().all(|key| !is_forbidden_key(key)));
        let parsed = ReplayFixture::parse(&json).unwrap();
        assert!(parsed.metadata.sanitized);
        parsed.replay().unwrap();
    }

    #[test]
    fn sanitizer_removes_normal_camel_snake_and_nested_secret_keys() {
        let mut value = serde_json::json!({
            "community": "super-secret-community",
            "authPassword": "hunter2",
            "nested": {
                "priv_password": "secret-privacy",
                "windowsPassword": "windows-secret",
                "arcAtlasToken": "atlas-secret",
                "api_key": "api-secret",
                "failure": "Authorization: Bearer bearer-secret"
            }
        });
        scrub_value(&mut value, None);
        let json = serde_json::to_string(&value).unwrap();
        for forbidden in [
            "super-secret-community",
            "hunter2",
            "secret-privacy",
            "windows-secret",
            "atlas-secret",
            "api-secret",
            "bearer-secret",
        ] {
            assert!(!json.contains(forbidden), "leaked {forbidden}: {json}");
        }
    }

    #[test]
    fn future_fixture_versions_are_rejected_clearly() {
        let error = ReplayFixture::parse(r#"{"fixtureVersion":99}"#).unwrap_err();
        assert!(matches!(error, ReplayError::UnsupportedVersion(99)));
        assert!(error.to_string().contains("supports version 1"));
    }

    #[test]
    fn expected_diff_includes_related_suppression() {
        let mut fixture = malicious_fixture();
        fixture.expected = Some(ReplayExpectations {
            links: vec![ExpectedLink {
                from_device_id: Some(1),
                to_device_id: Some(2),
                to_unresolved_id: None,
                from_port: Some("Port 12".into()),
                to_port: None,
                protocol: Some("fdb".into()),
                confidence: Some(TopologyConfidence::Strong),
                ..ExpectedLink::default()
            }],
            ..ReplayExpectations::default()
        });
        let result = fixture.replay().unwrap();
        let diff = compare_expected(&fixture, &result);
        assert!(diff.contains("REMOVED"));
        assert!(diff.contains("Port 12"));
    }

    #[test]
    fn expected_link_diff_reports_vlan_speed_and_poe_changes() {
        let capture = rich_capture();
        let mut fixture = ReplayFixture::from_capture(&capture, "mutated link expectations");
        let result = fixture.replay().unwrap();
        fixture.expected = Some(ReplayExpectations {
            links: vec![ExpectedLink {
                from_device_id: Some(1),
                to_device_id: Some(9),
                from_port: Some("WAN".into()),
                kind: Some("wireless".into()),
                speed_mbps: Some(100),
                vlan: Some("10".into()),
                native_vlan: Some(99),
                tagged_vlans: Some(vec![40]),
                poe_enabled: Some(false),
                poe_watts: Some(1.0),
                ..ExpectedLink::default()
            }],
            ..ReplayExpectations::default()
        });

        let diff = compare_expected(&fixture, &result);
        assert!(diff.contains("CHANGED"));
        for field in [
            "kind",
            "speedMbps",
            "vlan",
            "nativeVlan",
            "taggedVlans",
            "poe.enabled",
            "poe.watts",
        ] {
            assert!(diff.contains(field), "missing {field} from diff:\n{diff}");
        }
    }

    #[test]
    fn expected_wan_diff_reports_gateway_via_confidence_and_uplink_changes() {
        let capture = rich_capture();
        let mut fixture = ReplayFixture::from_capture(&capture, "mutated WAN expectations");
        let result = fixture.replay().unwrap();
        fixture.expected = Some(ReplayExpectations {
            wan: Some(ExpectedWan::Facts(ExpectedWanFacts {
                present: Some(true),
                gateway_device_id: Some(999),
                gateway_ip: Some("198.51.100.1".into()),
                gateway_mac: Some("00:FF:FF:00:00:01".into()),
                via_device_id: Some(998),
                confidence: Some(TopologyConfidence::Inferred),
                uplink_protocol: Some("manual".into()),
                uplink_from_port: Some("Internet".into()),
                uplink_to_port: Some("WAN".into()),
                ..ExpectedWanFacts::default()
            })),
            ..ReplayExpectations::default()
        });

        let diff = compare_expected(&fixture, &result);
        assert!(diff.contains("CHANGED\nWAN"));
        for field in [
            "gatewayDeviceId",
            "gatewayIp",
            "gatewayMac",
            "viaDeviceId",
            "confidence",
            "uplink.protocol",
            "uplink.fromPort",
            "uplink.toPort",
        ] {
            assert!(diff.contains(field), "missing {field} from diff:\n{diff}");
        }
    }

    #[test]
    fn replay_uses_the_same_correlation_result_as_live_evidence() {
        let capture = rich_capture();
        let (mut live_snapshot, _) = crate::correlate::correlate_detailed(
            &capture.views,
            &capture.targets,
            &capture.captured_at,
            capture.edge_hint.as_ref(),
        );
        normalize_snapshot(&mut live_snapshot);
        let fixture = ReplayFixture::from_capture(&capture, "same-engine check");
        assert!(!fixture.metadata.sanitized);
        assert!(serde_json::to_string(&fixture)
            .unwrap()
            .contains("\"sanitized\":false"));
        let json = fixture.to_sanitized_json(None).unwrap();
        assert!(json.contains("\"sanitized\": true"));
        let parsed = ReplayFixture::parse(&json).unwrap();
        assert!(parsed.metadata.sanitized);
        let first = parsed.replay().unwrap();
        let second = parsed.replay().unwrap();
        assert_eq!(first.snapshot, live_snapshot);
        let lldp = first
            .snapshot
            .connections
            .iter()
            .find(|link| link.protocol == "lldp")
            .unwrap();
        assert_eq!(lldp.vlan.as_deref(), Some("trunk"));
        assert_eq!(lldp.native_vlan, Some(10));
        assert_eq!(lldp.tagged_vlans, vec![20, 30]);
        assert_eq!(lldp.poe.as_ref().and_then(|poe| poe.watts), Some(8.2));
        assert_eq!(first.snapshot.edge.as_ref().unwrap().via_device_id, Some(9));
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
        assert_eq!(first.summary.duration_ms, 0);
    }
}
