# ArcScan topology discovery (v1.9 engine)

Topology discovery for the integrated v1.9 workflow. It does **not** rewrite
device classification or the inventory exporter. Quick Scan does not run it;
the technician starts it explicitly from the completed scan's Topology panel.

The callable surface is the Tauri commands in `src-tauri/src/topology/mod.rs`
and the `arcscan-topology` crate under `src-tauri/topology-engine/`. The live
ArcAtlas builder in `src/lib/arcatlas.ts` combines the returned snapshot with
the exact JSON rows produced by `buildInventoryExport`.

## What this engine produces

`TopologySnapshot` is the **internal** ArcScan UI payload. It may contain
unresolved neighbours as `unknownNodes` and connections with only one
inventory endpoint.

The schemaVersion 2 **handoff** surface is stricter, matching ArcAtlas-Next #13:

- `topology.connections` contain only known-to-known links.
- Both `fromDeviceId` and `toDeviceId` exist exactly once in the same
  `inventory` array.
- Unresolved-id fields are omitted from those connections.
- Unresolved evidence is preserved on the additive `unresolvedTopology`
  object so it is not discarded. The current ArcAtlas receiver ignores
  unknown fields.

`fromDeviceId` / `toDeviceId` are ArcScan **local inventory ids** for
correlation inside the same payload. They are not ArcAtlas canonical ids.

- `confidence` is one of `confirmed`, `strong`, `inferred`. Several weak clues
  never vote an inferred link up to confirmed.
- Unknown or unmanaged neighbours are preserved internally. No vendor or
  model is invented.

The end-to-end integration fixture is
`src/lib/fixtures/v1.9Integration.ts`, exercised by
`src/lib/v1.9Integration.test.ts`. The topology crate retains its lower-level
serializer fixture for isolated engine tests.

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

- `authPriv` and `authNoPriv` (the Topology panel exposes None (authNoPriv);
  `noAuthNoPriv` is refused)
- Optional SNMP context name
- Auth: MD5, SHA-1, SHA-224, SHA-256, SHA-384, SHA-512
- Privacy: DES, AES-128, AES-192, AES-256
- Engine-id discovery via `snmp2::SyncSession::init`
- Library errors are rewritten so a password cannot leak through `Display`

Known library limits, isolated behind `V3Session`:

- `snmp2` is a synchronous client. Each v3 GET/GETBULK runs on Tokio's
  blocking pool as **one** operation. Walks check cancel/deadline between
  rounds. The JoinHandle is always awaited, so a cancelled run does not
  return while USM work is still issuing packets.
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

LLDP/CDP neighbours resolve to inventory devices by **management IP or chassis
MAC only**. Hostname / sysName / CDP device-id is evidence for an unknown
node and never a canonical match. Duplicate hostnames stay unresolved unless
IP or MAC disambiguates them.

## Performance

- Per-device SNMP timeout (default 800 ms, clamped 100–5000)
- Per-device collect budget (8 s)
- Site wall clock (45 s)
- Concurrency 8 (max 16)
- Max 512 targets
- Cancel via `cancel_topology` and via the in-flight scan Stop hook. Stop and
  the site wall are noticed while probes are in flight (not only after the
  current future completes). In-flight SNMPv3 blocking work is joined before
  the run reports `cancelled` / `timedOut`.

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

## Port labels (issue #46)

SNMP `DisplayString` / `OctetString` values are decoded in
`topology-engine/src/display.rs`. `from_utf8_lossy` is not used: replacement
characters (U+FFFD) never become a primary port label.

Preference order for a physical port name:

1. `ifName`
2. `ifAlias`
3. `ifDescr`
4. numeric `ifIndex`

LLDP `locPortNum` and BRIDGE-MIB port numbers are used to resolve the correct
IF-MIB `ifIndex`. They are not a competing label: when locPort text is `7`
and IF-MIB `ifName` is `Gi1/0/7`, the displayed port is `Gi1/0/7`.

Latin-1 / Windows-1252 labels that are mostly printable ASCII are recovered
(so `Café-uplink` stays useful). NUL-padded and UTF-16 ASCII port names are
accepted. Byte strings that are mostly non-ASCII junk — the Netgear
`�=ü)` class — are rejected and noted internally as hex. The next IF-MIB
candidate is used instead.

FDB / MAC-table links keep the switch-side port and leave `toPort` empty when
the endpoint does not advertise one. Multi-MAC uplink/trunk FDB evidence is
not promoted to a direct endpoint link.

## WAN / Internet

The engine correlates the scanner's OS default route (Windows included,
`netinfo::default_gateway_ip`) plus any scan `scope_hint` gateway IP/MAC
against inventory.

- IP and MAC agree → `strong`
- Only one side matches → `inferred`
- IP and MAC point at different inventory devices → no edge
- No match → no Internet node and no invented gateway

When a gateway is identified, ArcScan adds a **logical** Internet node
(`logical:internet`, `physical: false`). It is never an inventory `device_id`
and is never written into the Inventory array. If a public IP / ASN / ISP
cannot be determined, the label is only `Internet`. An ONT/modem is kept
between Internet and the firewall only when LLDP/CDP evidence actually names
one. Nothing fabricates an ISP handoff device. A discovered ONT/modem already
present in inventory is kept as `edge.viaDeviceId` (Internet → ONT → gateway).
An unmanaged ONT stays `edge.viaUnresolvedId`. Both are additive; WAN uplinks
still do not enter canonical `topology.connections`.

WAN links use `kind: "wan"` and `protocol: "default-route"`. They live on the
additive `edge` object (and as unresolved evidence) so schemaVersion 2
`topology.connections` stays known-to-known inventory ids only.

## In-app preview

The Topology panel draws a lightweight SVG hierarchy:

Internet → WAN / ONT → firewall/router → switches → APs/servers/NAS → endpoints

It is confirmation, not a documentation editor. Fit, Zoom, Hide endpoints and
Reset layout are the only chrome. Confirmed links are solid and thicker,
strong links are solid and thinner, inferred links are dashed. Click, hover, or
tab to a connection and press Enter or Space for ports, protocol, confidence,
speed, VLAN, PoE and the first evidence line.

Inventory rows that share an explicit non-empty `physical_device` key render as
**one** node, with every distinct switch link kept. That grouping is preview-only:
transport-local inventory ids and the ArcAtlas handoff payload stay unmerged.
Hostname-only duplicates stay separate. Blank or absent physical-device keys do
not group.

A device whose inventory type is unknown is not guessed from its name. If it
sourced FDB/BRIDGE evidence (it published a MAC table), the preview may show a
presentation-only `Switch · FDB` role and place it on the switch layer. That
label does not write a classification into inventory.

## Integrated contract invariants

- Do not fold unresolved connections into `topology.connections` for ArcAtlas.
  Use `unresolvedTopology` until the receiver is extended.
- Additive schemaVersion 2 fields `edge` and `logicalNodes` carry the WAN /
  Internet presentation. Current ArcAtlas receivers ignore unknown keys.
- Device ids must be the same inventory ids the deep-discovery branch emits
  in the same payload.
- Quick Scan must stay fast. Topology stays opt-in / credentialed.
- Windows WMI/WinRM is Claude's lane and is unused here.

