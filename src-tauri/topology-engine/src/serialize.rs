//! Deterministic JSON for [`TopologySnapshot`] matching issue #42.

use super::model::{
    PoeInfo, TopologyConfidence, TopologyConnection, TopologyHandoffPreview, TopologySnapshot,
};

pub const SCHEMA_VERSION: u32 = 2;

/// Canonical JSON (pretty-printed, sorted object keys via serde's field order
/// which is the struct declaration order, connections already sorted).
pub fn snapshot_to_json(snapshot: &TopologySnapshot) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(snapshot)
}

pub fn handoff_preview_to_json(
    preview: &TopologyHandoffPreview,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(preview)
}

/// The issue #42 example, as a golden fixture. Inventory is empty because
/// this PR does not rewrite the existing exporter.
pub fn issue42_fixture() -> TopologyHandoffPreview {
    TopologyHandoffPreview {
        schema_version: SCHEMA_VERSION,
        handoff_id: "00000000-0000-4000-8000-000000000042".into(),
        source_version: env!("CARGO_PKG_VERSION").into(),
        generated_at: "2026-09-16T12:00:00Z".into(),
        network_name: "Site LAN".into(),
        inventory: Vec::new(),
        topology: TopologySnapshot {
            captured_at: "2026-09-16T12:00:00Z".into(),
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
            unknown_nodes: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializer_matches_issue_42_contract() {
        let json = handoff_preview_to_json(&issue42_fixture()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["schemaVersion"], 2);
        assert_eq!(value["networkName"], "Site LAN");
        assert!(value["inventory"].as_array().unwrap().is_empty());
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
        // Additive unknown-node fields must be omitted on a known-to-known link.
        assert!(conn.get("fromUnresolvedId").is_none());
        assert!(conn.get("toUnresolvedId").is_none());
        // Credentials must never appear.
        let dumped = json.to_ascii_lowercase();
        assert!(!dumped.contains("community"));
        assert!(!dumped.contains("password"));
        assert!(!dumped.contains("public"));
        assert!(!dumped.contains("private"));
    }

    #[test]
    fn fixture_round_trips() {
        let original = issue42_fixture();
        let json = handoff_preview_to_json(&original).unwrap();
        let parsed: TopologyHandoffPreview = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }
}
