use std::path::PathBuf;

use arcscan_topology::assert_topology_fixture;

#[test]
fn topology_fixtures() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let names = [
        "lldp-confirmed-neighbour.json",
        "cdp-confirmed-neighbour.json",
        "netgear-fdb-only.json",
        "multi-mac-uplink-suppression.json",
        "hostname-only-unresolved-neighbour.json",
        "duplicate-device-identity.json",
        "self-loop-evidence.json",
        "partial-snmp-mib-failure.json",
        "wan-direct-gateway.json",
        "wan-unresolved-ont.json",
        "wan-known-ont.json",
    ];
    for name in names {
        assert_topology_fixture(root.join(name));
    }
}
