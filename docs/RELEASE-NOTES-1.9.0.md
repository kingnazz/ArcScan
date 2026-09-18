# ArcScan 1.9.0

ArcScan 1.9.0 adds deep device discovery and an evidence-based network topology workflow while keeping Quick Scan fast and uncredentialed.

## Added

### Deep discovery

- **Deep Scan** can inspect already-discovered services for richer identity without changing the normal Quick Scan path.
- Optional **credentialed Windows discovery** can retrieve exact Windows product, edition, release/build, architecture, domain/workgroup, manufacturer/model, serial, SMBIOS system UUID, and workstation/server/domain-controller role when the technician supplies session credentials.
- Strong physical-device identifiers can reconcile multiple network interfaces as one machine without deleting or merging the underlying inventory rows.
- Management controllers remain distinct from their host servers.

### Topology discovery

- Optional post-scan topology discovery using **SNMP v2c or SNMPv3** credentials supplied by the technician.
- LLDP/CDP neighbour evidence, IF-MIB port names, BRIDGE/Q-BRIDGE forwarding tables, ARP support, VLAN/native/tagged VLAN data, link speed and PoE evidence where devices expose it.
- Conservative confidence levels:
  - **Confirmed** for direct LLDP/CDP neighbour evidence.
  - **Strong** for stable single-MAC switch/FDB evidence.
  - **Inferred** only where the evidence supports a best-effort relationship.
- No automatic community spraying. ArcScan never tries `public`, `private` or vendor defaults unless the technician explicitly enters them.
- SNMP and Windows credentials remain **session-only** and are not serialized into exports or ArcAtlas handoffs.

### Topology preview

- A lightweight hierarchical preview shows Internet/WAN, gateway, switches, infrastructure and endpoints.
- Switch-side port labels prefer **ifName → ifAlias → ifDescr → numeric ifIndex** and reject malformed SNMP display strings rather than rendering replacement-character garbage.
- FDB endpoint relationships keep the proven switch port and leave the endpoint-side port unknown unless it was actually advertised.
- Multiple inventory interfaces sharing one explicit physical-device identity can render as one preview node while retaining every distinct link and local inventory ID.
- Unknown devices that demonstrably source FDB/BRIDGE evidence can be shown as a presentation-only **Switch · FDB** role without changing inventory classification.
- Connections are keyboard operable and expose protocol, confidence, speed, VLAN, PoE and supporting evidence.

### WAN and ArcAtlas handoff

- ArcScan correlates the scanner's default route with inventory to present a conservative **Internet → gateway** relationship.
- A discovered ONT/modem is only placed between Internet and the gateway when direct neighbour evidence supports it. ArcScan does not guess an ISP.
- ArcAtlas schema v2 handoff includes known-to-known topology connections while preserving unresolved evidence separately.
- Additive `edge` and `logicalNodes` metadata carries Internet/WAN presentation without turning logical Internet into a scanned physical device.
- Inventory-only sends remain schema v1 for backward compatibility.

## Verification

The release candidate was exercised on a real managed Netgear network in addition to automated CI. The real-network test verified clean switch port labels, FDB fan-out, default-gateway/Internet presentation, conservative unknown-device handling, and topology preview behavior.

Project CI covers frontend typechecking/tests/build, browser and accessibility checks, Rust formatting/tests/Clippy, Portable edition checks, dependency audits, and Windows x64, Windows ARM64 and universal macOS installer builds.
