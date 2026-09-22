import { useState } from "react";
import { Button } from "../ui/primitives";
import {
  DIAGNOSTICS_EXPORT_WARNING,
  formatDiagnosticsExport,
  snmpStatusLabel,
  type TopologyDiagnostics,
} from "../lib/topology";
import { deviceCompactFacts, TopologyDeviceDiagnostics } from "./TopologyDeviceDiagnostics";

export function TopologyDiagnosticsPanel({
  diagnostics,
  onExportReplay,
}: {
  diagnostics: TopologyDiagnostics;
  onExportReplay?: () => Promise<boolean> | boolean;
}) {
  const [openIp, setOpenIp] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<string | null>(null);
  const summary = diagnostics.runSummary;
  const facts = [
    ["Devices queried", summary.devicesQueried],
    ["Devices responding", summary.devicesResponding],
    ["LLDP/CDP neighbours", summary.lldpCdpNeighbours],
    ["FDB relationships", summary.fdbRelationships],
    ["Confirmed links", summary.confirmedLinks],
    ["Strong links", summary.strongLinks],
    ["Unresolved neighbours", summary.unresolvedNeighbours],
    ["Suppressed candidates", summary.suppressedCandidates],
    ["Partial SNMP coverage", summary.partialSnmpDevices],
  ] as const;

  const copy = async () => {
    const text = formatDiagnosticsExport(diagnostics);
    try {
      await navigator.clipboard.writeText(text);
      setCopyState("Copied.");
    } catch {
      setCopyState("Clipboard unavailable. Use download instead.");
    }
  };

  const download = () => {
    const text = formatDiagnosticsExport(diagnostics);
    const blob = new Blob([text], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "arcscan-topology-diagnostics.json";
    anchor.click();
    URL.revokeObjectURL(url);
    setCopyState("Downloaded.");
  };

  const exportReplay = async () => {
    if (!onExportReplay) return;
    try {
      const written = await onExportReplay();
      if (written) setCopyState("Replay fixture exported.");
    } catch (error) {
      setCopyState(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <div className="mt-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 className="text-[12px] font-semibold text-text">Topology diagnostics</h4>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant="secondary" onClick={() => void copy()}>
            Copy diagnostics JSON
          </Button>
          <Button size="sm" variant="ghost" onClick={download}>
            Download diagnostics JSON
          </Button>
          {onExportReplay ? (
            <Button size="sm" variant="ghost" onClick={() => void exportReplay()}>
              Export replay fixture
            </Button>
          ) : null}
        </div>
      </div>
      <p className="mt-1 text-xs leading-relaxed text-text-muted">{DIAGNOSTICS_EXPORT_WARNING}</p>
      {onExportReplay ? (
        <p className="mt-1 text-xs leading-relaxed text-text-muted">
          Replay fixtures are machine-readable parsed evidence for developers. They contain network
          inventory information, never credentials, and nothing is uploaded automatically. Export
          before forgetting the session credentials so ArcScan can perform its final secret scrub.
        </p>
      ) : null}
      {copyState ? <p className="mt-1 text-xs text-text-secondary">{copyState}</p> : null}

      <dl className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
        {facts.map(([label, value]) => (
          <div key={label} className="rounded-md border border-border px-2 py-1.5">
            <dt className="text-[11px] text-text-muted">{label}</dt>
            <dd className="text-[13px] font-medium text-text">{value}</dd>
          </div>
        ))}
      </dl>

      <ul className="mt-3 divide-y divide-border">
        {diagnostics.devices.map((device) => {
          const open = openIp === device.targetIp;
          const name = device.displayName || device.sysName;
          const titled = name && name !== device.targetIp;
          return (
            <li key={device.targetIp} className="py-2">
              <button
                type="button"
                className="w-full text-left"
                aria-expanded={open}
                onClick={() => setOpenIp(open ? null : device.targetIp)}
              >
                {titled ? <span className="block text-[13px] font-medium text-text">{name}</span> : null}
                <span className={`mono text-xs ${titled ? "text-text-secondary" : "text-[13px] font-medium text-text"}`}>
                  {device.targetIp}
                </span>
                <span className="mt-1 block text-xs text-text-muted">
                  {snmpStatusLabel(device.snmpStatus)}
                  {" · "}
                  {deviceCompactFacts(device).join(" · ")}
                </span>
              </button>
              {open ? <TopologyDeviceDiagnostics device={device} /> : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
