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
use super::vlan::{is_valid_vlan_id, normalize_vlan_ids, normalize_vlan_label, TRUNK_LABEL};

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
            unresolved_links.push(conn.with_normalized_vlans());
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
        edge: snapshot.edge.clone(),
        logical_nodes: snapshot.logical_nodes.clone(),
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
        // Last gate before the wire. ArcAtlas rejects the whole handoff over
        // one VLAN outside 1..=4094, so an unusable VLAN is dropped here and
        // the link is still handed over.
        vlan: normalize_vlan_label(conn.vlan.as_deref()),
        native_vlan: conn.native_vlan.filter(|v| is_valid_vlan_id(*v)),
        tagged_vlans: normalize_vlan_ids(conn.tagged_vlans.iter().copied()),
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
        edge: None,
        logical_nodes: Vec::new(),
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
        assert_contract_vlans(conn)?;
    }
    for row in inventory {
        if row.get("id").and_then(|v| v.as_str()) == Some(crate::display::INTERNET_NODE_ID)
            || row.get("kind").and_then(|v| v.as_str()) == Some("internet")
        {
            return Err("inventory must not contain the logical Internet node".into());
        }
    }
    Ok(())
}

/// ArcAtlas accepts VLAN IDs in `1..=4094` only, and rejects the entire
/// handoff when one connection carries anything else. Checked on the
/// serialized form, because that is what ArcAtlas actually parses.
fn assert_contract_vlans(conn: &serde_json::Value) -> Result<(), String> {
    if let Some(value) = conn.get("nativeVlan").filter(|v| !v.is_null()) {
        let vlan = value
            .as_u64()
            .ok_or_else(|| format!("nativeVlan {value} is not a VLAN id"))?;
        if u16::try_from(vlan).map(is_valid_vlan_id) != Ok(true) {
            return Err(format!("nativeVlan {vlan} is outside the 1-4094 contract"));
        }
    }
    if let Some(value) = conn.get("taggedVlans").filter(|v| !v.is_null()) {
        let tagged = value
            .as_array()
            .ok_or_else(|| "taggedVlans must be an array".to_string())?;
        for entry in tagged {
            let vlan = entry
                .as_u64()
                .ok_or_else(|| format!("taggedVlans entry {entry} is not a VLAN id"))?;
            if u16::try_from(vlan).map(is_valid_vlan_id) != Ok(true) {
                return Err(format!(
                    "taggedVlans entry {vlan} is outside the 1-4094 contract"
                ));
            }
        }
    }
    if let Some(value) = conn.get("vlan").filter(|v| !v.is_null()) {
        let label = value
            .as_str()
            .ok_or_else(|| format!("vlan {value} is not a label"))?;
        if label != TRUNK_LABEL && normalize_vlan_label(Some(label)).is_none() {
            return Err(format!("vlan {label:?} is outside the 1-4094 contract"));
        }
    }
    Ok(())
}

/// A handoff whose VLAN facts sit exactly on the contract edges: no native
/// VLAN at all, and tagged VLANs including both `1` and `4094`. Keeps the
/// boundary of what ArcAtlas accepts pinned to something ArcScan can serialize.
pub fn vlan_contract_fixture() -> TopologyHandoffPreview {
    let inventory = vec![
        serde_json::json!({
            "device_id": 1,
            "device_name": "Core Switch",
            "current_ip": "192.168.1.2",
            "mac": "00:20:AA:00:00:02",
            "hostname": "core-sw",
            "presence": "present"
        }),
        serde_json::json!({
            "device_id": 2,
            "device_name": "Access Switch",
            "current_ip": "192.168.1.3",
            "mac": "00:20:AA:00:00:03",
            "hostname": "access-sw",
            "presence": "present"
        }),
    ];
    let snapshot = TopologySnapshot {
        captured_at: "2026-09-22T12:00:00Z".into(),
        connections: vec![TopologyConnection {
            from_device_id: Some(1),
            to_device_id: Some(2),
            from_unresolved_id: None,
            to_unresolved_id: None,
            from_logical_id: None,
            to_logical_id: None,
            from_port: Some("Gi1/0/1".into()),
            to_port: Some("Gi1/0/24".into()),
            kind: "ethernet".into(),
            protocol: "lldp".into(),
            confidence: TopologyConfidence::Confirmed,
            speed_mbps: Some(1000),
            vlan: Some(TRUNK_LABEL.into()),
            // No native VLAN: the trunk reported none, which is a fact ArcScan
            // records as unknown rather than inventing one.
            native_vlan: None,
            tagged_vlans: vec![1, 100, 4094],
            poe: None,
            evidence: vec![
                "LLDP neighbour on Gi1/0/1: chassis 00:20:AA:00:00:03, remote port Gi1/0/24".into(),
            ],
        }],
        unknown_nodes: Vec::new(),
        logical_nodes: Vec::new(),
        edge: None,
    };
    preview_from_snapshot(
        &snapshot,
        inventory,
        "vlan-contract-fixture",
        "ArcScan VLAN contract",
        "2026-09-22T12:00:05Z",
    )
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

    /// A link carrying every kind of unusable VLAN fact, so the serializer is
    /// tested on the shape it actually has to defend against.
    fn link_with_vlans(
        vlan: Option<&str>,
        native_vlan: Option<u16>,
        tagged_vlans: Vec<u16>,
    ) -> TopologyConnection {
        TopologyConnection {
            from_device_id: Some(2),
            to_device_id: Some(1),
            from_unresolved_id: None,
            to_unresolved_id: None,
            from_logical_id: None,
            to_logical_id: None,
            from_port: Some("Gi1/0/48".into()),
            to_port: Some("X0".into()),
            kind: "ethernet".into(),
            protocol: "cdp".into(),
            confidence: TopologyConfidence::Confirmed,
            speed_mbps: Some(1000),
            vlan: vlan.map(str::to_string),
            native_vlan,
            tagged_vlans,
            poe: None,
            evidence: vec!["CDP neighbour on Gi1/0/48".into()],
        }
    }

    fn preview_with(connections: Vec<TopologyConnection>) -> TopologyHandoffPreview {
        let snapshot = TopologySnapshot {
            captured_at: "2026-09-22T12:00:00Z".into(),
            connections,
            unknown_nodes: vec![],
            logical_nodes: vec![],
            edge: None,
        };
        preview_from_snapshot(
            &snapshot,
            vec![
                serde_json::json!({"device_id": 1, "device_name": "Firewall"}),
                serde_json::json!({"device_id": 2, "device_name": "Core Switch"}),
            ],
            "handoff-vlan",
            "Site LAN",
            "2026-09-22T12:00:05Z",
        )
    }

    /// The compatibility failure the audit found, end to end on the wire: a
    /// CDP neighbour reported native VLAN 0, and that one fact used to make
    /// ArcAtlas reject the whole handoff.
    #[test]
    fn a_native_vlan_of_zero_is_absent_and_the_link_is_still_handed_over() {
        let preview = preview_with(vec![link_with_vlans(Some("0"), Some(0), vec![])]);
        assert_eq!(preview.topology.connections.len(), 1);
        let link = &preview.topology.connections[0];
        assert_eq!(link.native_vlan, None);
        assert_eq!(link.vlan, None);
        assert_eq!(link.from_device_id, 2);
        assert_eq!(link.to_device_id, 1);
        assert_eq!(link.from_port.as_deref(), Some("Gi1/0/48"));
        assert_eq!(link.confidence, TopologyConfidence::Confirmed);

        let json = handoff_preview_to_json(&preview).unwrap();
        assert_arc_atlas13_contract(&json).unwrap();
        // Absent, not null and not 0: `nativeVlan` is skipped when unknown.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let conn = &value["topology"]["connections"][0];
        assert!(conn.get("nativeVlan").is_none(), "{conn}");
        assert!(conn.get("vlan").is_none(), "{conn}");
        assert!(!json.contains("\"nativeVlan\": 0"));
    }

    #[test]
    fn the_contract_edges_reach_the_wire_unchanged() {
        let preview = preview_with(vec![link_with_vlans(
            Some("trunk"),
            Some(1),
            vec![1, 100, 4094],
        )]);
        let link = &preview.topology.connections[0];
        assert_eq!(link.native_vlan, Some(1));
        assert_eq!(link.tagged_vlans, vec![1, 100, 4094]);
        let json = handoff_preview_to_json(&preview).unwrap();
        assert_arc_atlas13_contract(&json).unwrap();

        let preview = preview_with(vec![link_with_vlans(Some("4094"), Some(4094), vec![])]);
        let link = &preview.topology.connections[0];
        assert_eq!(link.native_vlan, Some(4094));
        assert_eq!(link.vlan.as_deref(), Some("4094"));
        assert_arc_atlas13_contract(&handoff_preview_to_json(&preview).unwrap()).unwrap();
    }

    #[test]
    fn the_serialized_handoff_carries_no_vlan_outside_the_contract() {
        let preview = preview_with(vec![
            link_with_vlans(Some("0"), Some(0), vec![0, 1, 100, 4094, 4095]),
            link_with_vlans(Some("4095"), Some(4095), vec![4095]),
            link_with_vlans(Some("65546"), Some(u16::MAX), vec![u16::MAX, 20]),
            link_with_vlans(Some("trunk"), Some(1), vec![1, 4094]),
        ]);
        let json = handoff_preview_to_json(&preview).unwrap();
        assert_arc_atlas13_contract(&json).unwrap();

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let connections = value["topology"]["connections"].as_array().unwrap();
        assert_eq!(connections.len(), 4, "no link was discarded over a VLAN");
        for conn in connections {
            if let Some(native) = conn.get("nativeVlan") {
                let native = native.as_u64().unwrap();
                assert!((1..=4094).contains(&native), "nativeVlan {native}");
            }
            for tagged in conn
                .get("taggedVlans")
                .and_then(|v| v.as_array())
                .unwrap_or(&Vec::new())
            {
                let tagged = tagged.as_u64().unwrap();
                assert!((1..=4094).contains(&tagged), "taggedVlans {tagged}");
            }
            if let Some(label) = conn.get("vlan").and_then(|v| v.as_str()) {
                if label != "trunk" {
                    let parsed: u64 = label.parse().unwrap();
                    assert!((1..=4094).contains(&parsed), "vlan {label}");
                }
            }
        }
        assert_eq!(
            connections[0]["taggedVlans"],
            serde_json::json!([1, 100, 4094])
        );
        assert_eq!(connections[2]["taggedVlans"], serde_json::json!([20]));
    }

    #[test]
    fn unresolved_links_are_normalized_too() {
        // `unresolvedTopology` rides the same envelope, so it gets the same
        // treatment even though ArcAtlas ignores it today.
        let mut orphan = link_with_vlans(Some("0"), Some(0), vec![0, 30, 4095]);
        orphan.to_device_id = None;
        orphan.to_unresolved_id = Some("unknown:chassis:deadbeef0001".into());
        let preview = preview_with(vec![orphan]);
        assert!(preview.topology.connections.is_empty());
        let unresolved = preview.unresolved_topology.as_ref().expect("kept");
        assert_eq!(unresolved.connections.len(), 1);
        assert_eq!(unresolved.connections[0].native_vlan, None);
        assert_eq!(unresolved.connections[0].vlan, None);
        assert_eq!(unresolved.connections[0].tagged_vlans, vec![30]);
        assert_eq!(
            unresolved.connections[0].to_unresolved_id.as_deref(),
            Some("unknown:chassis:deadbeef0001")
        );
    }

    #[test]
    fn the_contract_assertion_rejects_an_out_of_range_vlan() {
        // The assertion is the gate the serializer is measured against, so it
        // has to fail on a payload the serializer could never produce.
        let preview = preview_with(vec![link_with_vlans(Some("trunk"), Some(10), vec![10])]);
        let mut value: serde_json::Value =
            serde_json::from_str(&handoff_preview_to_json(&preview).unwrap()).unwrap();
        value["topology"]["connections"][0]["nativeVlan"] = serde_json::json!(0);
        let err = assert_arc_atlas13_contract(&value.to_string()).unwrap_err();
        assert!(err.contains("nativeVlan"), "{err}");

        value["topology"]["connections"][0]["nativeVlan"] = serde_json::json!(10);
        value["topology"]["connections"][0]["taggedVlans"] = serde_json::json!([10, 4095]);
        let err = assert_arc_atlas13_contract(&value.to_string()).unwrap_err();
        assert!(err.contains("taggedVlans"), "{err}");

        value["topology"]["connections"][0]["taggedVlans"] = serde_json::json!([10]);
        value["topology"]["connections"][0]["vlan"] = serde_json::json!("4095");
        let err = assert_arc_atlas13_contract(&value.to_string()).unwrap_err();
        assert!(err.contains("vlan"), "{err}");
    }

    #[test]
    fn the_vlan_contract_fixture_sits_on_the_contract_edges() {
        let fixture = vlan_contract_fixture();
        assert_eq!(fixture.schema_version, SCHEMA_VERSION);
        assert_eq!(fixture.topology.connections.len(), 1);
        let link = &fixture.topology.connections[0];
        assert_eq!(link.native_vlan, None, "native VLAN is unset, not invented");
        assert_eq!(link.vlan.as_deref(), Some("trunk"));
        assert_eq!(link.tagged_vlans, vec![1, 100, 4094]);
        assert!(link.tagged_vlans.contains(&1));
        assert!(link.tagged_vlans.contains(&4094));

        let json = handoff_preview_to_json(&fixture).unwrap();
        assert_arc_atlas13_contract(&json).unwrap();
        assert!(!json.contains("nativeVlan"), "unset stays absent");
        let parsed: TopologyHandoffPreview = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, fixture);
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
                    from_logical_id: None,
                    to_logical_id: None,
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
                    from_logical_id: None,
                    to_logical_id: None,
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
            logical_nodes: vec![],
            edge: None,
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
                from_logical_id: None,
                to_logical_id: None,
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
            logical_nodes: vec![],
            edge: None,
        };
        let (topology, unresolved) = split_for_contract(&snapshot, &[]);
        assert!(topology.connections.is_empty());
        let unresolved = unresolved.expect("internal evidence kept");
        assert_eq!(unresolved.connections.len(), 1);
        let preview = preview_from_snapshot(&snapshot, vec![], "h", "n", "t");
        let json = handoff_preview_to_json(&preview).unwrap();
        assert_arc_atlas13_contract(&json).unwrap();
    }

    #[test]
    fn wan_edge_is_additive_and_internet_is_not_inventory() {
        use crate::correlate::correlate_with_edge;
        use crate::model::{EdgeHint, TopologyTarget};
        use std::net::Ipv4Addr;

        let mut view = crate::collect::DeviceView::new(Ipv4Addr::new(192, 168, 1, 1), Some(1));
        view.sys_name = Some("firewall".into());
        let targets = vec![TopologyTarget {
            ip: "192.168.1.1".into(),
            mac: Some("00:20:AA:00:00:01".into()),
            device_id: Some(1),
            hostname: Some("fw".into()),
            detected_name: Some("Firewall".into()),
        }];
        let snapshot = correlate_with_edge(
            &[view],
            &targets,
            "t",
            Some(&EdgeHint {
                gateway_ip: Some("192.168.1.1".into()),
                gateway_mac: Some("00:20:AA:00:00:01".into()),
            }),
        );
        let inventory = vec![serde_json::json!({
            "device_id": 1,
            "device_name": "Firewall",
            "current_ip": "192.168.1.1"
        })];
        let preview = preview_from_snapshot(&snapshot, inventory, "h", "n", "t");
        assert!(preview.edge.is_some());
        assert_eq!(preview.logical_nodes.len(), 1);
        assert!(!preview.logical_nodes[0].physical);
        assert!(preview.topology.connections.iter().all(|c| c.kind != "wan"));
        let json = handoff_preview_to_json(&preview).unwrap();
        assert_arc_atlas13_contract(&json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["schemaVersion"], 2);
        assert_eq!(
            value["edge"]["internet"]["id"],
            crate::display::INTERNET_NODE_ID
        );
        assert_eq!(value["edge"]["internet"]["physical"], false);
        assert_eq!(value["logicalNodes"][0]["kind"], "internet");
        let inventory = value["inventory"].as_array().unwrap();
        assert!(inventory
            .iter()
            .all(|row| row.get("device_id").and_then(|v| v.as_i64()) == Some(1)));
        let dumped = json.to_ascii_lowercase();
        assert!(!dumped.contains("community"));
        assert!(!dumped.contains("password"));
    }
}
