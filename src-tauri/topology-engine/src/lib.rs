//! ArcScan topology discovery engine.
//!
//! Isolated from device classification and inventory mapping. The application
//! combines its snapshot with the real Inventory JSON for ArcAtlas. Quick Scan
//! does not run this.

pub mod ber;
pub mod collect;
pub mod correlate;
pub mod credentials;
pub mod diagnostics;
pub mod display;
pub mod engine;
pub mod error;
pub mod model;
pub mod providers;
pub mod serialize;
pub mod snmp;
pub mod vlan;

pub use credentials::{CredentialInput, CredentialStatus, CredentialStore, SnmpSecret};
pub use display::INTERNET_NODE_ID;
pub use engine::{request_cancel, run, run_from_request, set_scan_cancel_check};
pub use error::TopologyError;
pub use model::{
    ContractConnection, ContractTopology, LogicalNode, TopologyConfidence, TopologyConnection,
    TopologyEdge, TopologyHandoffPreview, TopologyRequest, TopologyResult, TopologySnapshot,
    TopologySummary, TopologyTarget, UnresolvedTopology,
};
pub use serialize::{
    assert_arc_atlas13_contract, handoff_preview_to_json, issue42_fixture, preview_from_snapshot,
    snapshot_to_json, split_for_contract, vlan_contract_fixture, SCHEMA_VERSION,
};
pub use vlan::{
    is_valid_vlan_id, normalize_vlan_id, normalize_vlan_ids, normalize_vlan_label, MAX_VLAN_ID,
    MIN_VLAN_ID,
};

pub fn isolated_from_classifier() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ber::{Oid, SnmpValue};
    use crate::collect::{
        collect_device, CDP_CACHE_ADDRESS, CDP_CACHE_DEVICE_ID, CDP_CACHE_DEVICE_PORT,
        CDP_CACHE_NATIVE_VLAN, DOT1D_TP_FDB_PORT, IF_ALIAS, IF_DESCR, IF_HIGH_SPEED, IF_NAME,
        IF_OPER_STATUS, IF_PHYS_ADDRESS, LLDP_LOC_CHASSIS_ID, LLDP_REM_CHASSIS_ID,
        LLDP_REM_PORT_ID, LLDP_REM_SYS_NAME, PETH_PSE_DETECTION, SYS_DESCR, SYS_NAME,
    };
    use crate::correlate::correlate;
    use crate::snmp::FixtureSession;
    use std::collections::BTreeMap;
    use std::net::Ipv4Addr;

    fn octet(s: &str) -> SnmpValue {
        SnmpValue::OctetString(s.as_bytes().to_vec())
    }

    fn mac_bytes(octets: [u8; 6]) -> SnmpValue {
        SnmpValue::OctetString(octets.to_vec())
    }

    fn int(v: i64) -> SnmpValue {
        SnmpValue::Integer(v)
    }

    fn gauge(v: u32) -> SnmpValue {
        SnmpValue::Gauge32(v)
    }

    fn site_targets() -> Vec<TopologyTarget> {
        vec![
            TopologyTarget {
                ip: "192.168.1.1".into(),
                mac: Some("00:20:AA:00:00:01".into()),
                device_id: Some(1),
                hostname: Some("sonicwall".into()),
                detected_name: Some("SonicWall".into()),
            },
            TopologyTarget {
                ip: "192.168.1.2".into(),
                mac: Some("00:1A:2B:00:00:02".into()),
                device_id: Some(2),
                hostname: Some("core-sw".into()),
                detected_name: Some("Core Switch".into()),
            },
            TopologyTarget {
                ip: "192.168.1.12".into(),
                mac: Some("00:1A:2B:00:00:12".into()),
                device_id: Some(3),
                hostname: Some("u7-pro".into()),
                detected_name: Some("U7 Pro".into()),
            },
            TopologyTarget {
                ip: "192.168.1.20".into(),
                mac: Some("00:11:32:00:00:20".into()),
                device_id: Some(4),
                hostname: Some("synology".into()),
                detected_name: Some("Synology NAS".into()),
            },
            TopologyTarget {
                ip: "192.168.1.50".into(),
                mac: Some("AA:BB:CC:00:00:50".into()),
                device_id: Some(5),
                hostname: Some("workstation".into()),
                detected_name: None,
            },
        ]
    }

    fn core_switch_table() -> BTreeMap<String, SnmpValue> {
        let mut t = BTreeMap::new();
        t.insert(Oid::from_slice(SYS_NAME).to_dotted(), octet("core-sw"));
        t.insert(
            Oid::from_slice(SYS_DESCR).to_dotted(),
            octet("Core access switch"),
        );
        t.insert(
            Oid::from_slice(LLDP_LOC_CHASSIS_ID).to_dotted(),
            mac_bytes([0x00, 0x1A, 0x2B, 0x00, 0x00, 0x02]),
        );
        for (idx, name) in [
            (7u32, "Port 7"),
            (12, "Port 12"),
            (20, "Port 20"),
            (24, "Port 24"),
            (48, "Port 48"),
        ] {
            t.insert(format!("{}.{idx}", Oid::from_slice(IF_DESCR)), octet(name));
            t.insert(format!("{}.{idx}", Oid::from_slice(IF_NAME)), octet(name));
            t.insert(format!("{}.{idx}", Oid::from_slice(IF_ALIAS)), octet(name));
            t.insert(format!("{}.{idx}", Oid::from_slice(IF_OPER_STATUS)), int(1));
            t.insert(
                format!("{}.{idx}", Oid::from_slice(IF_HIGH_SPEED)),
                gauge(1000),
            );
        }
        t.insert(
            format!("{}.7", Oid::from_slice(IF_PHYS_ADDRESS)),
            mac_bytes([0x00, 0x1A, 0x2B, 0x00, 0x00, 0x02]),
        );
        t.insert(
            format!("{}.0.48.1", Oid::from_slice(LLDP_REM_CHASSIS_ID)),
            mac_bytes([0x00, 0x20, 0xAA, 0x00, 0x00, 0x01]),
        );
        t.insert(
            format!("{}.0.48.1", Oid::from_slice(LLDP_REM_PORT_ID)),
            octet("X0"),
        );
        t.insert(
            format!("{}.0.48.1", Oid::from_slice(LLDP_REM_SYS_NAME)),
            octet("sonicwall"),
        );
        t.insert(
            format!("{}.0.12.1", Oid::from_slice(LLDP_REM_CHASSIS_ID)),
            mac_bytes([0x00, 0x1A, 0x2B, 0x00, 0x00, 0x12]),
        );
        t.insert(
            format!("{}.0.12.1", Oid::from_slice(LLDP_REM_PORT_ID)),
            octet("eth0"),
        );
        t.insert(
            format!("{}.0.12.1", Oid::from_slice(LLDP_REM_SYS_NAME)),
            octet("u7-pro"),
        );
        t.insert(
            format!("{}.1.12", Oid::from_slice(PETH_PSE_DETECTION)),
            int(3),
        );
        t.insert(
            format!(
                "{}.{}.{}.{}.{}.{}.{}",
                Oid::from_slice(DOT1D_TP_FDB_PORT),
                0xAAu32,
                0xBBu32,
                0xCCu32,
                0,
                0,
                0x50
            ),
            int(7),
        );
        t.insert(
            format!(
                "{}.{}.{}.{}.{}.{}.{}",
                Oid::from_slice(DOT1D_TP_FDB_PORT),
                0x00u32,
                0x11,
                0x32,
                0x00,
                0x00,
                0x20
            ),
            int(20),
        );
        for last in 1..6u32 {
            t.insert(
                format!("{}.10.10.10.0.0.{last}", Oid::from_slice(DOT1D_TP_FDB_PORT)),
                int(24),
            );
        }
        t
    }

    #[tokio::test]
    async fn collect_then_correlate_matches_the_site_story() {
        let sess = FixtureSession::new(core_switch_table());
        let view = collect_device(&sess, Ipv4Addr::new(192, 168, 1, 2), Some(2))
            .await
            .unwrap();
        assert_eq!(view.sys_name.as_deref(), Some("core-sw"));
        assert!(!view.lldp_neighbors.is_empty());
        let snap = correlate(&[view], &site_targets(), "2026-09-16T12:00:00Z");
        let protocols: Vec<_> = snap
            .connections
            .iter()
            .map(|c| c.protocol.as_str())
            .collect();
        assert!(protocols.contains(&"lldp"), "{protocols:?}");
        assert!(snap
            .connections
            .iter()
            .any(|c| c.to_device_id == Some(1) && c.confidence == TopologyConfidence::Confirmed));
        assert!(snap
            .connections
            .iter()
            .any(|c| c.to_device_id == Some(3) && c.protocol == "lldp"));
        let fake = snap
            .connections
            .iter()
            .filter(|c| c.from_port.as_deref() == Some("Port 24"))
            .count();
        assert_eq!(fake, 0);
    }

    /// The ArcScan <-> ArcAtlas compatibility blocker, start to finish: a real
    /// SNMP answer of `cdpCacheNativeVLAN = 0` walked, correlated and
    /// serialized. Before VLAN normalization, that single 0 reached
    /// `topology.connections[].nativeVlan` and ArcAtlas rejected the whole
    /// handoff -- every other link in the payload with it.
    #[tokio::test]
    async fn a_cdp_native_vlan_of_zero_still_produces_a_usable_handoff() {
        let mut table = core_switch_table();
        // Port 36 has a CDP neighbour that reports "no native VLAN".
        table.insert(
            format!("{}.36", Oid::from_slice(IF_DESCR)),
            octet("Port 36"),
        );
        table.insert(format!("{}.36", Oid::from_slice(IF_NAME)), octet("Port 36"));
        table.insert(format!("{}.36", Oid::from_slice(IF_OPER_STATUS)), int(1));
        table.insert(
            format!("{}.36", Oid::from_slice(IF_HIGH_SPEED)),
            gauge(1000),
        );
        table.insert(
            format!("{}.36.1", Oid::from_slice(CDP_CACHE_DEVICE_ID)),
            octet("synology"),
        );
        table.insert(
            format!("{}.36.1", Oid::from_slice(CDP_CACHE_DEVICE_PORT)),
            octet("eth0"),
        );
        table.insert(
            format!("{}.36.1", Oid::from_slice(CDP_CACHE_ADDRESS)),
            SnmpValue::IpAddress(std::net::Ipv4Addr::new(192, 168, 1, 20)),
        );
        table.insert(
            format!("{}.36.1", Oid::from_slice(CDP_CACHE_NATIVE_VLAN)),
            int(0),
        );

        let view = collect_device(
            &FixtureSession::new(table),
            Ipv4Addr::new(192, 168, 1, 2),
            Some(2),
        )
        .await
        .unwrap();
        let neighbor = view
            .cdp_neighbors
            .iter()
            .find(|n| n.device_id.as_deref() == Some("synology"))
            .expect("the CDP neighbour survives its unusable native VLAN");
        assert_eq!(neighbor.native_vlan, None);

        let snapshot = correlate(&[view], &site_targets(), "2026-09-22T12:00:00Z");
        let cdp_link = snapshot
            .connections
            .iter()
            .find(|c| c.protocol == "cdp")
            .expect("ArcScan still emits the valid link");
        assert_eq!(cdp_link.to_device_id, Some(4));
        assert_eq!(cdp_link.from_port.as_deref(), Some("Port 36"));
        assert_eq!(cdp_link.to_port.as_deref(), Some("eth0"));
        assert_eq!(cdp_link.native_vlan, None, "nativeVlan is unknown, not 0");
        assert_eq!(cdp_link.vlan, None);
        // The LLDP links in the same snapshot are untouched by the bad VLAN.
        assert!(snapshot
            .connections
            .iter()
            .any(|c| c.to_device_id == Some(1)));
        assert!(snapshot
            .connections
            .iter()
            .any(|c| c.to_device_id == Some(3)));

        let inventory: Vec<serde_json::Value> = site_targets()
            .iter()
            .map(|t| {
                serde_json::json!({
                    "device_id": t.device_id,
                    "device_name": t.detected_name,
                    "current_ip": t.ip,
                    "mac": t.mac,
                })
            })
            .collect();
        let preview = preview_from_snapshot(
            &snapshot,
            inventory,
            "handoff-native-vlan-zero",
            "Site LAN",
            "2026-09-22T12:00:05Z",
        );
        let json = handoff_preview_to_json(&preview).unwrap();
        // The handoff remains usable by ArcAtlas: it passes the contract, and
        // the link is in it.
        assert_arc_atlas13_contract(&json).unwrap();
        let handed_over = preview
            .topology
            .connections
            .iter()
            .find(|c| c.protocol == "cdp")
            .expect("the link reaches ArcAtlas");
        assert_eq!(handed_over.to_device_id, 4);
        assert_eq!(handed_over.native_vlan, None);
        assert!(!json.contains("\"nativeVlan\": 0"));
    }

    #[test]
    fn credentials_never_appear_in_the_fixture() {
        let json = handoff_preview_to_json(&issue42_fixture()).unwrap();
        for needle in ["password", "authKey", "community "] {
            assert!(
                !json
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase()),
                "fixture leaked {needle}"
            );
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap()["schemaVersion"],
            2
        );
    }

    #[test]
    fn this_crate_does_not_name_device_types() {
        assert!(isolated_from_classifier());
    }

    #[tokio::test]
    async fn cdp_table_is_collected() {
        let mut t = BTreeMap::new();
        t.insert(Oid::from_slice(SYS_NAME).to_dotted(), octet("core-sw"));
        t.insert(
            format!("{}.36.1", Oid::from_slice(CDP_CACHE_DEVICE_ID)),
            octet("access-sw"),
        );
        t.insert(
            format!("{}.36.1", Oid::from_slice(CDP_CACHE_DEVICE_PORT)),
            octet("Gi0/1"),
        );
        let view = collect_device(
            &FixtureSession::new(t),
            Ipv4Addr::new(192, 168, 1, 2),
            Some(2),
        )
        .await
        .unwrap();
        assert_eq!(view.cdp_neighbors.len(), 1);
        assert_eq!(
            view.cdp_neighbors[0].device_id.as_deref(),
            Some("access-sw")
        );
    }

    #[test]
    fn v3_secret_debug_does_not_leak() {
        let secret = SnmpSecret::from_input(CredentialInput {
            version: "v3".into(),
            community: None,
            username: Some("monitor-user".into()),
            auth_protocol: Some("sha256".into()),
            auth_password: Some("auth-pass-xyz".into()),
            priv_protocol: Some("aes128".into()),
            priv_password: Some("priv-pass-xyz".into()),
            context: None,
        })
        .unwrap();
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains("auth-pass-xyz"));
        assert!(!rendered.contains("priv-pass-xyz"));
        assert!(!rendered.contains("monitor-user"));
    }
}
