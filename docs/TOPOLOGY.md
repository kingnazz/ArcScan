# ArcScan topology discovery (v1.9 engine)

Standalone topology engine for issue #42. It does **not** rewrite device
classification, the inventory exporter, or the current ArcAtlas handoff
envelope. Quick Scan does not run it.

The callable surface is the Tauri commands in `src-tauri/src/topology/mod.rs`
and the `arcscan-topology` crate under `src-tauri/topology-engine/`. Final
ArcAtlas integration (schemaVersion 2 handoff with inventory filled in) is
deferred until the Claude deep-discovery branch and this branch are reviewed
together.

## What this engine produces

`TopologySnapshot` matches the additive contract in issue #42:

- `fromDeviceId` / `toDeviceId` are ArcScan **local inventory ids** for
  correlation inside the same payload. They are not ArcAtlas canonical ids.
- `confidence` is one of `confirmed`, `strong`, `inferred`. Several weak clues
  never vote an inferred link up to confirmed.
- Unknown or unmanaged neighbours are preserved as `unknownNodes` with an
  `unknown:…` id. No vendor or model is invented.

The golden serializer fixture is `topology_contract_fixture` /
`arcscan_topology::issue42_fixture()`.

## Protocols and MIBs

| Area | Status |
| --- | --- |
| SNMPv2c GET / GETBULK | In-tree Tokio UDP client (`topology-engine/src/snmp.rs`, `ber.rs`) |
| SNMPv3 USM auth/privacy | `snmp2` 0.5 with `crypto-rust` (not OpenSSL) |
| LLDP-MIB | Collected |
| CISCO-CDP-MIB | Collected when present |
| IF-MIB (name, oper status, speed) | Collected |
| BRIDGE-MIB / Q-BRIDGE-MIB FDB | Collected |
| IP-MIB ARP / neighbour | Collected as supporting evidence on FDB links |
| Q-BRIDGE VLAN / PVID | Collected (access vs trunk) |
| POWER-ETHERNET-MIB + Cisco watts | Collected when present |
| ENTITY-MIB model/mfg | Collected when present; first row, not chassis-resolved |

UniFi controller API and SonicWall API are named on the provider trait and
**not implemented**. Standards first.

## SNMPv3

Implemented, not faked:

- `authPriv` and `authNoPriv`
- Auth: MD5, SHA-1, SHA-224, SHA-256, SHA-384, SHA-512
- Privacy: DES, AES-128, AES-192, AES-256
- `noAuthNoPriv` is refused
- Engine-id discovery via `snmp2::SyncSession::init`
- Library errors are rewritten so a password cannot leak through `Display`

Known library limits, isolated behind `V3Session`:

- `snmp2` is a synchronous client, so each v3 operation runs on Tokio's
  blocking pool rather than on the async scanner runtime.
- Context names are forwarded when the technician supplies one.
- Informs/traps are out of scope.

If a future `snmp2` release changes USM, only `snmp.rs` should move.

## Security model

- Credentials are technician-entered. Nothing tries `public`, `private`, or
  vendor defaults.
- Session-only process memory. Not written to disk, keyring, or ArcAtlas.
- Status DTOs report that credentials exist and which version/protocols were
  chosen. They never return the community, username, or passwords.
- Error strings are sanitized. A broken device is isolated; it does not hang
  the site run and it does not invite spraying another community.

## Confidence

- `confirmed` — LLDP or CDP neighbour declaration
- `strong` — exactly one relevant unicast MAC on a stable access port, plus
  ARP when it agrees
- `inferred` — reserved; this PR does not mint inferred physical links from
  weak agreement. Unmanaged intermediates are `unknownNodes` instead.

LLDP/CDP beat FDB on the same port. Two switches reporting each other collapse
to one undirected connection.

A port with two or more learned unicast MACs is treated as an uplink/trunk and
does **not** produce endpoint links.

## Performance

- Per-device SNMP timeout (default 800 ms, clamped 100–5000)
- Per-device collect budget (8 s)
- Site wall clock (45 s)
- Concurrency 8 (max 16)
- Max 512 targets
- Cancel via `cancel_topology` and via the in-flight scan Stop hook

## Vendor limits worth knowing at integration time

- LLDP `lldpLocPortNum` is not always `ifIndex`. Many Cisco boxes differ;
  the engine uses the local port table when it is present and otherwise the
  numeric index.
- PoE group/port → ifIndex mapping is vendor-specific. Watts come from
  Cisco POWER-ETHERNET-EXT when that table answers; IEEE detection status
  alone only yields enabled/disabled.
- ENTITY-MIB often lists fans and PSUs first. The first model string is
  recorded as metadata, never as a classified device type.
- Consumer gateways frequently answer IF-MIB and ignore LLDP/BRIDGE. That
  is a successful partial collect with zero links, not a failure.

## Integration notes for the Claude + Grok merge

- Do not fold this snapshot into the current inventory exporter yet.
- Device ids must be the same inventory ids the deep-discovery branch emits
  in the same payload.
- Quick Scan must stay fast. Topology stays opt-in / credentialed.
- Windows WMI/WinRM is Claude's lane and is unused here.
