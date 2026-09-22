// The ArcScan <-> ArcAtlas VLAN contract, as fixtures.
//
// ArcAtlas accepts Ethernet VLAN IDs in 1-4094 only. These two snapshots pin
// both ends of that rule: one sits exactly on the contract edges, and one
// carries every kind of VLAN fact ArcScan has to normalize away before a
// handoff. Both reference the device ids in `V19_INTEGRATION_ROWS`.

import type { TopologySnapshot } from "../topology";

const CAPTURED_AT = "2026-09-22T12:00:00.000Z";

/**
 * The contract edges: no native VLAN at all (the trunk reported none, which
 * stays unknown rather than being invented), and tagged VLANs including both
 * boundary values, 1 and 4094.
 */
export const VLAN_CONTRACT_TOPOLOGY: TopologySnapshot = {
  capturedAt: CAPTURED_AT,
  connections: [
    {
      fromDeviceId: 40,
      toDeviceId: 31,
      fromPort: "Gi1/0/1",
      toPort: "eth0",
      kind: "ethernet",
      protocol: "lldp",
      confidence: "confirmed",
      speedMbps: 1000,
      vlan: "trunk",
      taggedVlans: [1, 100, 4094],
      evidence: ["LLDP neighbour declaration"],
    },
  ],
  unknownNodes: [],
};

/**
 * The compatibility failure the audit found, plus the neighbouring shapes: a
 * CDP neighbour reporting native VLAN 0 ("no native VLAN"), an out-of-range
 * 4095, a value wide enough to truncate into a plausible VLAN under a raw
 * `as u16`, and one ordinary link that must be unaffected by any of them.
 */
export const VLAN_OUT_OF_RANGE_TOPOLOGY: TopologySnapshot = {
  capturedAt: CAPTURED_AT,
  connections: [
    {
      fromDeviceId: 40,
      toDeviceId: 31,
      fromPort: "Gi1/0/36",
      toPort: "eth0",
      kind: "ethernet",
      protocol: "cdp",
      confidence: "confirmed",
      speedMbps: 1000,
      vlan: "0",
      nativeVlan: 0,
      taggedVlans: [],
      evidence: ["CDP neighbour on Gi1/0/36"],
    },
    {
      fromDeviceId: 40,
      toDeviceId: 22,
      fromPort: "Gi1/0/20",
      toPort: "Ethernet 2",
      kind: "ethernet",
      protocol: "lldp",
      confidence: "confirmed",
      speedMbps: 1000,
      vlan: "4095",
      nativeVlan: 4095,
      taggedVlans: [0, 1, 100, 4094, 4095],
      evidence: ["LLDP neighbour declaration"],
    },
    {
      fromDeviceId: 40,
      toDeviceId: 21,
      fromPort: "Gi1/0/21",
      toPort: "Ethernet 1",
      kind: "ethernet",
      protocol: "lldp",
      confidence: "confirmed",
      speedMbps: 1000,
      vlan: "65546",
      nativeVlan: 65546,
      taggedVlans: [65546, 20],
      evidence: ["LLDP neighbour declaration"],
    },
    {
      fromDeviceId: 40,
      toDeviceId: 30,
      fromPort: "Gi1/0/30",
      toPort: "eth0",
      kind: "ethernet",
      protocol: "lldp",
      confidence: "confirmed",
      speedMbps: 1000,
      vlan: "10",
      nativeVlan: 10,
      taggedVlans: [10, 20],
      evidence: ["LLDP neighbour declaration"],
    },
  ],
  unknownNodes: [],
};
