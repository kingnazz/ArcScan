//! Deterministic JSON for the schemaVersion 2 handoff contract (issue #42 /
//! ArcAtlas-Next #13).
//!
//! Internal [`TopologySnapshot`] values keep unresolved-node evidence for the
//! ArcScan UI. The contract serializer exposes only known-to-known connections
//! whose `fromDeviceId` / `toDeviceId` exist exactly once in `inventory`.
//! Remaining evidence is preserved on the additive `unresolvedTopology` field
//! so it is not discarded, and so the current ArcAtlas receiver can ignore it.

use std::collections::HashMap;

use super::model::{
    ContractConnection, ContractTopology, PoeInfo, TopologyConfidence, TopologyConnection,
    TopologyHandoffPreview, TopologySnapshot, UnresolvedTopology,
};

pub const SCHEMA_VERSION: u32 = 2;

/// Canonical JSON (pretty-printed, serde field order).
pub fn snapshot_to_json(snapshot: &TopologySnapshot) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(snapshot)
}

pub fn handoff_preview_to_json(
    preview: &TopologyHandoffPreview,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(preview)
}

/// Split an internal snapshot into the locked issue #42 topology object plus
/// an additive unresolved extension. Unresolved evidence is never deleted.
pub fn split_for_contract(
    snapshot: &TopologySnapshot,
    inventory: &[serde_json::Value],
) -> (ContractTopology, Option<UnresolvedTopology>) {
    let counts = inventory_id_counts(inventory);
    let mut contract_links = Vec::new();
    let mut unresolved_links = Vec::new();
    for conn in &snapshot.connections {
        if let Some(link) = contract_connection(conn, &counts) {
            contract_links.push(link);
        } else {
            unresolved_links.push(conn.clone());
        }
    }
    let unresolved = if snapshot.unknown_nodes.is_empty() && unresolved_links.is_empty() {
        None
    } else {
        Some(UnresolvedTopology {
            unknown_nodes: snapshot.unknown_nodes.clone(),
            connections: unresolved_links,
        })
    };
    (
        ContractTopology {
            captured_at: snapshot.captured_at.clone(),
            connections: contract_links,
        },
        unresolved,
    )
}

pub fn preview_from_snapshot(
    snapshot: &TopologySnapshot,
    inventory: Vec<serde_json::Value>,
    handoff_id: impl Into<String>,
    network_name: impl Into<String>,
    generated_at: impl Into<String>,
) -> TopologyHandoffPreview {
    let (topology, unresolved_topology) = split_for_contract(snapshot, &inventory);
    TopologyHandoffPreview {
        schema_version: SCHEMA_VERSION,
        handoff_id: handoff_id.into(),
        source_version: env!("CARGO_PKG_VERSION").into(),
        generated_at: generated_at.into(),
        network_name: network_name.into(),
        inventory,
        topology,
        unresolved_topology,
    }
}

fn inventory_id_counts(inventory: &[serde_json::Value]) -> HashMap<i64, usize> {
    let mut counts = HashMap::new();
    for row in inventory {
        if let Some(id) = row
            .get("device_id")
            .or_else(|| row.get("deviceId"))
            .and_then(serde_json::Value::as_i64)
        {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    counts
}

fn contract_connection(
    conn: &TopologyConnection,
    counts: &HashMap<i64, usize>,
) -> Option<ContractConnection> {
    let from = conn.from_device_id?;
    let to = conn.to_device_id?;
    if counts.get(&from).copied() != Some(1) || counts.get(&to).copied() != Some(1) {
        return None;
    }
    Some(ContractConnection {
        from_device_id: from,
        to_device_id: to,
        from_port: conn.from_port.clone(),
        to_port: conn.to_port.clone(),
        kind: conn.kind.clone(),
        protocol: conn.protocol.clone(),
        confidence: conn.confidence,
        speed_mbps: conn.speed_mbps,
        vlan: conn.vlan.clone(),
        native_vlan: conn.native_vlan,
        tagged_vlans: conn.tagged_vlans.clone(),
        poe: conn.poe.clone(),
        evidence: conn.evidence.clone(),
    })
}

/// The issue #42 example, as a golden fixture. Inventory contains the two
/// endpoint rows the ArcAtlas #13 parser requires so both IDs exist exactly
/// once; it is not produced by the live exporter.
pub fn issue42_fixture() -> TopologyHandoffPreview {
    let inventory = vec![
        serde_json::json!({
            "device_id": 1,
            "device_name": "SonicWall",
            "current_ip": "192.168.1.1",
            "mac": "00:20:AA:00:00:01",
            "hostname": "sonicwall",
            "presence": "present"
        }),
        serde_json::json!({
            "device_id": 2,
            "device_name": "Core Switch",
            "current_ip": "192.168.1.2",
            "mac": "00:1A:2B:00:00:02",
            "hostname": "core-sw",
            "presence": "present"
        }),
    ];
    TopologyHandoffPreview {
        schema_version: SCHEMA_VERSION,
        handoff_id: "00000000-0000-4000-8000-000000000042".into(),
        source_version: env!("CARGO_PKG_VERSION").into(),
        generated_at: "2026-09-16T12:00:00Z".into(),
        network_name: "Site LAN".into(),
        inventory,
        topology: ContractTopology {
            captured_at: "2026-09-16T12:00:00Z".into(),
            connections: vec![ContractConnection {
                from_device_id: 2,
                to_device_id: 1,
                from_port: Some("48".into()),
                to_port: Some("X0".into()),
                kind: "ethernet".into(),
                protocol: "lldp".into(),
                confidence: TopologyConfidence::Confirmed,
                speed_mbps: Some(1000),
                vlan: Some("trunk".into()),
                native_vlan: Some(10),
                tagged_vlans: vec![10, 20, 30],
                poe: Some(PoeInfo {
                    enabled: true,
                    watts: Some(8.2),
                }),
                evidence: vec!["LLDP neighbor on Core Switch port 48 reports SonicWall X0".into()],
            }],
        },
        unresolved_topology: None,
    }
}

/// ArcAtlas-Next #13 parser assumptions: every serialized connection endpoint
/// exists exactly once in the serialized inventory, and no unresolved-id
/// fields appear inside `topology.connections`.
pub fn assert_arc_atlas13_contract(json: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("invalid json: {e}"))?;
    let inventory = value
        .get("inventory")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "inventory must be an array".to_string())?;
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for row in inventory {
        let id = row
            .get("device_id")
            .or_else(|| row.get("deviceId"))
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| "inventory row missing device_id".to_string())?;
        *counts.entry(id).or_insert(0) += 1;
    }
    let connections = value
        .pointer("/topology/connections")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "topology.connections must be an array".to_string())?;
    for conn in connections {
        if conn.get("fromUnresolvedId").is_some() || conn.get("toUnresolvedId").is_some() {
            return Err(
                "topology.connections must not carry unresolved-id fields on the contract surface"
                    .into(),
            );
        }
        let from = conn
            .get("fromDeviceId")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| "fromDeviceId must be a number".to_string())?;
        let to = conn
            .get("toDeviceId")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| "toDeviceId must be a number".to_string())?;
        if counts.get(&from).copied() != Some(1) {
            return Err(format!(
                "fromDeviceId {from} does not exist exactly once in inventory"
            ));
        }
        if counts.get(&to).copied() != Some(1) {
            return Err(format!(
                "toDeviceId {to} does not exist exactly once in inventory"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TopologyConnection, UnresolvedNode};

    #[test]
    fn serializer_matches_issue_42_contract() {
        let json = handoff_preview_to_json(&issue42_fixture()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["schemaVersion"], 2);
        assert_eq!(value["networkName"], "Site LAN");
        assert_eq!(value["inventory"].as_array().unwrap().len(), 2);
        let conn = &value["topology"]["connections"][0];
        assert_eq!(conn["fromDeviceId"], 2);
        assert_eq!(conn["toDeviceId"], 1);
        assert_eq!(conn["fromPort"], "48");
        assert_eq!(conn["toPort"], "X0");
        assert_eq!(conn["kind"], "ethernet");
        assert_eq!(conn["protocol"], "lldp");
        assert_eq!(conn["confidence"], "confirmed");
        assert_eq!(conn["speedMbps"], 1000);
        assert_eq!(conn["vlan"], "trunk");
        assert_eq!(conn["nativeVlan"], 10);
        assert_eq!(conn["taggedVlans"], serde_json::json!([10, 20, 30]));
        assert_eq!(conn["poe"]["enabled"], true);
        assert_eq!(conn["poe"]["watts"], 8.2);
        assert!(conn["evidence"][0].as_str().unwrap().contains("LLDP"));
        assert!(conn.get("fromUnresolvedId").is_none());
        assert!(conn.get("toUnresolvedId").is_none());
        assert!(value["topology"].get("unknownNodes").is_none());
        assert!(value.get("unresolvedTopology").is_none());
        let dumped = json.to_ascii_lowercase();
        assert!(!dumped.contains("community"));
        assert!(!dumped.contains("password"));
        assert!(!dumped.contains("public"));
        assert!(!dumped.contains("private"));
        assert_arc_atlas13_contract(&json).unwrap();
    }

    #[test]
    fn fixture_round_trips() {
        let original = issue42_fixture();
        let json = handoff_preview_to_json(&original).unwrap();
        let parsed: TopologyHandoffPreview = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn unresolved_evidence_is_preserved_outside_contract_connections() {
        let snapshot = TopologySnapshot {
            captured_at: "2026-09-16T12:00:00Z".into(),
            connections: vec![
                TopologyConnection {
                    from_device_id: Some(2),
                    to_device_id: Some(1),
                    from_unresolved_id: None,
                    to_unresolved_id: None,
                    from_port: Some("48".into()),
                    to_port: Some("X0".into()),
                    kind: "ethernet".into(),
                    protocol: "lldp".into(),
                    confidence: TopologyConfidence::Confirmed,
                    speed_mbps: Some(1000),
                    vlan: Some("trunk".into()),
                    native_vlan: Some(10),
                    tagged_vlans: vec![10, 20, 30],
                    poe: None,
                    evidence: vec!["LLDP neighbor".into()],
                },
                TopologyConnection {
                    from_device_id: Some(2),
                    to_device_id: None,
                    from_unresolved_id: None,
                    to_unresolved_id: Some("unknown:chassis:deadbeef0001".into()),
                    from_port: Some("36".into()),
                    to_port: Some("Gi0/1".into()),
                    kind: "ethernet".into(),
                    protocol: "lldp".into(),
                    confidence: TopologyConfidence::Confirmed,
                    speed_mbps: Some(1000),
                    vlan: None,
                    native_vlan: None,
                    tagged_vlans: vec![],
                    poe: None,
                    evidence: vec!["LLDP neighbour mystery-sw".into()],
                },
            ],
            unknown_nodes: vec![UnresolvedNode {
                id: "unknown:chassis:deadbeef0001".into(),
                chassis_id: Some("DE:AD:BE:EF:00:01".into()),
                sys_name: Some("mystery-sw".into()),
                management_address: None,
                reason: "LLDP neighbour is not present in this scan's inventory.".into(),
                source: "lldp".into(),
            }],
        };
        let inventory = vec![
            serde_json::json!({"device_id": 1, "device_name": "Firewall"}),
            serde_json::json!({"device_id": 2, "device_name": "Core Switch"}),
        ];
        let preview = preview_from_snapshot(
            &snapshot,
            inventory,
            "handoff-1",
            "Site LAN",
            "2026-09-16T12:00:00Z",
        );
        assert_eq!(preview.topology.connections.len(), 1);
        assert_eq!(preview.topology.connections[0].from_device_id, 2);
        assert_eq!(preview.topology.connections[0].to_device_id, 1);
        let unresolved = preview.unresolved_topology.as_ref().expect("kept");
        assert_eq!(unresolved.unknown_nodes.len(), 1);
        assert_eq!(unresolved.connections.len(), 1);
        assert_eq!(
            unresolved.connections[0].to_unresolved_id.as_deref(),
            Some("unknown:chassis:deadbeef0001")
        );

        let json = handoff_preview_to_json(&preview).unwrap();
        assert_arc_atlas13_contract(&json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["topology"]["connections"].as_array().unwrap().len(),
            1
        );
        assert!(value["topology"].get("unknownNodes").is_none());
        assert_eq!(
            value["unresolvedTopology"]["unknownNodes"][0]["sysName"],
            "mystery-sw"
        );
        assert_eq!(snapshot.unknown_nodes.len(), 1);
        assert_eq!(snapshot.connections.len(), 2);
    }

    #[test]
    fn empty_inventory_cannot_emit_known_links() {
        let snapshot = TopologySnapshot {
            captured_at: "t".into(),
            connections: vec![TopologyConnection {
                from_device_id: Some(2),
                to_device_id: Some(1),
                from_unresolved_id: None,
                to_unresolved_id: None,
                from_port: Some("48".into()),
                to_port: Some("X0".into()),
                kind: "ethernet".into(),
                protocol: "lldp".into(),
                confidence: TopologyConfidence::Confirmed,
                speed_mbps: None,
                vlan: None,
                native_vlan: None,
                tagged_vlans: vec![],
                poe: None,
                evidence: vec!["LLDP".into()],
            }],
            unknown_nodes: vec![],
        };
        let (topology, unresolved) = split_for_contract(&snapshot, &[]);
        assert!(topology.connections.is_empty());
        let unresolved = unresolved.expect("internal evidence kept");
        assert_eq!(unresolved.connections.len(), 1);
        let preview = preview_from_snapshot(&snapshot, vec![], "h", "n", "t");
        let json = handoff_preview_to_json(&preview).unwrap();
        assert_arc_atlas13_contract(&json).unwrap();
    }
}
