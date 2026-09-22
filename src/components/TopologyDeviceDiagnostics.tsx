import {
  mibStateLabel,
  neighbourFact,
  type DeviceTopologyDiagnostics,
  type MibState,
} from "../lib/topology";

function stateText(state: MibState | string): string {
  return mibStateLabel(state);
}

export function TopologyDeviceDiagnostics({ device }: { device: DeviceTopologyDiagnostics }) {
  const sections: Array<{ title: string; body: string[] }> = [
    {
      title: "Discovery",
      body: [
        device.failureReason ?? "",
        device.sysName ? `sysName ${device.sysName}` : "",
        ...device.mibCoverage.map(
          (item) =>
            `${item.mib}: ${stateText(item.state)}${item.rows ? ` · ${item.rows} rows` : ""}${
              item.detail ? ` · ${item.detail}` : ""
            }`,
        ),
        ...device.notes,
        ...device.hints,
      ].filter(Boolean),
    },
    {
      title: "Interfaces",
      body: [
        `${device.interfaces.count} interfaces`,
        `${device.interfaces.up} up`,
        `${device.interfaces.withName} named`,
        `${device.interfaces.withSpeed} with a reported speed`,
      ],
    },
    {
      title: "LLDP/CDP",
      body: [
        `LLDP ${neighbourFact(device.lldp.neighbourCount, device.lldp.state, "No neighbours")} · ${device.lldp.resolved} resolved`,
        `CDP ${neighbourFact(device.cdp.neighbourCount, device.cdp.state, "No neighbours")} · ${device.cdp.resolved} resolved`,
        ...device.lldp.neighbours.map(
          (neighbour) =>
            `LLDP ${neighbour.localPort} → ${neighbour.remotePort ?? "remote port unknown"} · ${
              neighbour.sysName ?? neighbour.chassisId ?? "unnamed"
            } · ${neighbour.resolution}`,
        ),
        ...device.cdp.neighbours.map(
          (neighbour) =>
            `CDP ${neighbour.localPort} → ${neighbour.remotePort ?? "remote port unknown"} · ${
              neighbour.sysName ?? "unnamed"
            } · ${neighbour.resolution}`,
        ),
      ],
    },
    {
      title: "FDB",
      body: [
        `${device.fdb.totalRows} rows · ${device.fdb.unicastMacs} relevant unicast · ${device.fdb.matchedInventory} matched inventory · ${device.fdb.unresolvedMacs} unresolved`,
        `${device.fdb.singleMacPorts} single-MAC ports · ${device.fdb.multiMacPorts} multi-MAC ports · ${device.fdb.strongLinks} strong links · ${device.fdb.uplinkSuppressions} uplink suppressions`,
        ...device.fdb.ports.map((port) => port.summary),
        device.fdb.portsOmitted > 0
          ? `${device.fdb.portsOmitted} additional ports are included in the counts only.`
          : "",
      ].filter(Boolean),
    },
    {
      title: "ARP",
      body: [
        `${stateText(device.arp.state)} · ${device.arp.entries} entries · ${device.arp.fdbCorroborations} corroborate an FDB link`,
      ],
    },
    {
      title: "VLAN",
      body: [
        `${stateText(device.vlan.state)} · ${device.vlan.accessPorts} access · ${device.vlan.trunkPorts} trunk · ${device.vlan.pvidPorts} PVID`,
      ],
    },
    {
      title: "PoE",
      body: [
        `Detection ${stateText(device.poe.detectionState)} · wattage ${stateText(device.poe.wattageState)}`,
        `${device.poe.enabledPorts} enabled · ${device.poe.portsWithWatts} with watts · ${device.poe.enabledWithoutWatts} enabled without watts`,
      ],
    },
    {
      title: "Relationships",
      body: [
        ...device.relationships.map((link) => link.why),
        device.relationshipsOmitted > 0
          ? `${device.relationshipsOmitted} additional relationships are counted above and omitted here.`
          : "",
        device.zeroLinkExplanation ?? "",
      ].filter(Boolean),
    },
    {
      title: "Suppressed evidence",
      body: [
        ...device.suppressions.map((item) => item.summary),
        device.suppressionsOmitted > 0
          ? `${device.suppressionsOmitted} additional suppressions are included in the run count only.`
          : "",
      ].filter(Boolean),
    },
  ];

  return (
    <div className="mt-2 space-y-3 border-t border-border pt-2">
      {device.portMappings.length > 0 ? (
        <div>
          <h5 className="text-[12px] font-semibold text-text">Port mapping</h5>
          <ul className="mt-1 space-y-1 text-xs leading-relaxed text-text-secondary">
            {device.portMappings.map((port) => (
              <li key={`${port.role}-${port.rawPort}-${port.resolvedIfIndex}`}>
                {port.role} raw {port.rawPort}
                {port.bridgePort != null ? ` · bridge ${port.bridgePort}` : ""} · ifIndex{" "}
                {port.resolvedIfIndex} · {port.displayLabel}. {port.resolutionSource}
                {port.fellBackToNumeric ? " Numeric fallback." : ""}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {sections.map((section) => (
        <div key={section.title}>
          <h5 className="text-[12px] font-semibold text-text">{section.title}</h5>
          {section.body.length === 0 ? (
            <p className="mt-1 text-xs text-text-muted">Nothing recorded.</p>
          ) : (
            <ul className="mt-1 space-y-1 text-xs leading-relaxed text-text-secondary">
              {section.body.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          )}
        </div>
      ))}
    </div>
  );
}

export function deviceCompactFacts(device: DeviceTopologyDiagnostics): string[] {
  const lldp =
    device.lldp.neighbourCount === 0 && device.lldp.state === "noRows"
      ? "No neighbours"
      : device.lldp.neighbourCount > 0
        ? `${device.lldp.neighbourCount} neighbours`
        : stateText(device.lldp.state);
  const cdp =
    device.cdp.neighbourCount === 0 && device.cdp.state === "noRows"
      ? "No neighbours"
      : device.cdp.neighbourCount > 0
        ? `${device.cdp.neighbourCount} neighbours`
        : stateText(device.cdp.state);
  const links = device.relationshipCount ?? device.relationships.length + device.relationshipsOmitted;
  return [
    `LLDP ${lldp}`,
    `CDP ${cdp}`,
    `FDB ${device.fdb.totalRows} MACs`,
    `ARP ${stateText(device.arp.state)}`,
    `VLAN ${stateText(device.vlan.state)}`,
    `PoE ${stateText(device.poe.detectionState)}`,
    `Links ${links}`,
    `Suppressed ${device.suppressions.length + device.suppressionsOmitted}`,
  ];
}

