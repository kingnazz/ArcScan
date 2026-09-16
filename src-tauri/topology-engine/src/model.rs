//! Topology domain model.
//!
//! This is the contract described in GitHub issue #42: an additive
//! `schemaVersion: 2` snapshot that the future ArcAtlas handoff can embed
//! without changing the existing inventory envelope. Device IDs are ArcScan
//! local inventory ids for correlation *inside the same payload*. They are not
//! ArcAtlas canonical ids.
//!
//! Unknown / unmanaged neighbours are preserved as unresolved nodes rather
//! than invented vendors or models.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// How sure ArcScan is about a physical or logical link.
///
/// These three words are the issue #42 vocabulary. They are not scores and
/// must never be averaged or "voted up": several `inferred` clues do not
/// become `confirmed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TopologyConfidence {
    /// Direct LLDP, CDP, or equivalent neighbour-protocol declaration.
    Confirmed,
    /// Strong SNMP/FDB/ARP correlation, but no neighbour protocol said so.
    Strong,
    /// Best-effort assumption. Display as inferred; never silently replace
    /// technician documentation.
    Inferred,
}

impl TopologyConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Strong => "strong",
            Self::Inferred => "inferred",
        }
    }

    /// Higher is more certain. Used only to *keep* the stronger of two
    /// observations of the same link, never to promote a weak one.
    pub fn rank(self) -> u8 {
        match self {
            Self::Inferred => 0,
            Self::Strong => 1,
            Self::Confirmed => 2,
        }
    }
}

impl PartialOrd for TopologyConfidence {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TopologyConfidence {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank().cmp(&other.rank())
    }
}

/// The neighbour protocol (or table) that produced a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TopologyProtocol {
    Lldp,
    Cdp,
    Fdb,
    Arp,
}

impl TopologyProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lldp => "lldp",
            Self::Cdp => "cdp",
            Self::Fdb => "fdb",
            Self::Arp => "arp",
        }
    }

    /// LLDP and CDP beat FDB/ARP when the same link is observed twice.
    pub fn rank(self) -> u8 {
        match self {
            Self::Arp => 0,
            Self::Fdb => 1,
            Self::Cdp => 2,
            Self::Lldp => 3,
        }
    }
}

/// PoE state collected from POWER-ETHERNET-MIB (and vendor extensions when
/// they actually report watts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoeInfo {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watts: Option<f64>,
}

/// A neighbour ArcScan can prove exists but cannot identify as an inventory
/// device. Never given a fabricated vendor or model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedNode {
    /// Stable within this snapshot, e.g. `unknown:chassis:aabbccddeeff`.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chassis_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sys_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub management_address: Option<String>,
    /// Why this was not matched to inventory. Plain language.
    pub reason: String,
    pub source: String,
}

/// One directed-or-undirected link in the topology snapshot.
///
/// `fromDeviceId` / `toDeviceId` match issue #42 when both ends resolved.
/// When an end is an unknown/unmanaged device, that id is `null` and the
/// corresponding `*UnresolvedId` points at [`TopologySnapshot::unknown_nodes`].
/// That pair of fields is additive; known-to-known connections serialize
/// exactly as the issue #42 example.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyConnection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_device_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_device_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_unresolved_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_unresolved_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_port: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_port: Option<String>,
    pub kind: String,
    pub protocol: String,
    pub confidence: TopologyConfidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_mbps: Option<u64>,
    /// `"trunk"` or a single VLAN id as a decimal string, matching issue #42.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_vlan: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tagged_vlans: Vec<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poe: Option<PoeInfo>,
    pub evidence: Vec<String>,
}

/// Standalone topology payload. Not embedded in the current ArcAtlas envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologySnapshot {
    pub captured_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<TopologyConnection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_nodes: Vec<UnresolvedNode>,
}

/// Future additive handoff shape from issue #42. Inventory is left empty here
/// on purpose: this crate does not rewrite the existing exporter. The final
/// integration fills `inventory` from the current mapper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyHandoffPreview {
    pub schema_version: u32,
    pub handoff_id: String,
    pub source_version: String,
    pub generated_at: String,
    pub network_name: String,
    pub inventory: Vec<serde_json::Value>,
    pub topology: TopologySnapshot,
}

/// What one topology run did, for the UI summary. Contains no secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologySummary {
    pub devices_queried: usize,
    pub devices_responded: usize,
    pub devices_failed: usize,
    pub confirmed: usize,
    pub strong: usize,
    pub inferred: usize,
    pub unknown_nodes: usize,
    pub duration_ms: u64,
    pub cancelled: bool,
    pub timed_out: bool,
    /// Sanitized per-device outcomes. Never includes credentials.
    pub failures: Vec<TopologyDeviceFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyDeviceFailure {
    pub ip: String,
    pub reason: String,
}

/// The value returned to the UI: snapshot + summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyResult {
    pub snapshot: TopologySnapshot,
    pub summary: TopologySummary,
}

/// One inventory device the correlator may attach a link to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyTarget {
    pub ip: String,
    #[serde(default)]
    pub mac: Option<String>,
    #[serde(default)]
    pub device_id: Option<i64>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub detected_name: Option<String>,
}

/// Request body for `discover_topology`. Credentials are *not* included:
/// they live in the session store so they cannot leak through this DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyRequest {
    pub targets: Vec<TopologyTarget>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub concurrency: Option<usize>,
    #[serde(default)]
    pub network_name: Option<String>,
    /// When set, Stop on the in-flight scan also stops topology.
    #[serde(default)]
    pub scan_id: Option<u64>,
}

impl TopologySnapshot {
    pub fn empty(captured_at: impl Into<String>) -> Self {
        Self {
            captured_at: captured_at.into(),
            connections: Vec::new(),
            unknown_nodes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_never_ranks_inferred_above_confirmed() {
        assert!(TopologyConfidence::Confirmed > TopologyConfidence::Strong);
        assert!(TopologyConfidence::Strong > TopologyConfidence::Inferred);
        assert_eq!(TopologyConfidence::Confirmed.as_str(), "confirmed");
    }

    #[test]
    fn lldp_outranks_fdb() {
        assert!(TopologyProtocol::Lldp.rank() > TopologyProtocol::Fdb.rank());
        assert!(TopologyProtocol::Cdp.rank() > TopologyProtocol::Fdb.rank());
    }
}
