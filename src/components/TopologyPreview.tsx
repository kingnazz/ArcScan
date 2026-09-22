// Lightweight topology preview. ArcAtlas still owns the documented map.
// This view only confirms that discovery produced a sensible hierarchy.

import { useMemo, useState } from "react";
import {
  Camera,
  Cloud,
  Focus,
  HardDrive,
  HelpCircle,
  Monitor,
  Network,
  Printer,
  RotateCcw,
  Router,
  Server,
  Shield,
  Wifi,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { Tooltip } from "../ui/Popover";
import { IconButton } from "../ui/primitives";
import {
  CONFIDENCE_HINT,
  PREVIEW_NODE_HEIGHT,
  PREVIEW_NODE_WIDTH,
  confidenceLabel,
  connectionDetailLines,
  layoutTopology,
  protocolLabel,
  speedLabel,
  vlanLabel,
  type DeviceNameLookup,
  type DeviceTypeLookup,
  type PhysicalDeviceLookup,
  type PreviewKind,
  type PreviewNode,
  whyForConnection,
  type TopologyConnection,
  type TopologyDiagnostics,
  type TopologySnapshot,
  type UnresolvedNode,
} from "../lib/topology";

export interface TopologyPreviewProps {
  snapshot: TopologySnapshot;
  names: DeviceNameLookup;
  types: DeviceTypeLookup;
  physical?: PhysicalDeviceLookup;
  diagnostics?: TopologyDiagnostics | null;
}

const KIND_LABEL: Record<PreviewKind, string> = {
  internet: "Internet",
  firewall: "Firewall",
  router: "Router",
  switch: "Switch",
  access_point: "Access point",
  server: "Server",
  domain_controller: "Domain controller",
  nas: "NAS",
  workstation: "Workstation",
  computer: "Computer",
  printer: "Printer",
  camera: "Camera",
  unknown: "Unknown",
};

function roleCaption(node: PreviewNode): string {
  const kind = KIND_LABEL[node.kind];
  if (node.roleSource === "topology") return `${kind} · FDB`;
  if (!node.physical) return `${kind} · logical`;
  return kind;
}

function KindIcon({ kind }: { kind: PreviewKind }) {
  const cls = "h-4 w-4";
  switch (kind) {
    case "internet":
      return <Cloud className={cls} aria-hidden />;
    case "firewall":
      return <Shield className={cls} aria-hidden />;
    case "router":
      return <Router className={cls} aria-hidden />;
    case "switch":
      return <Network className={cls} aria-hidden />;
    case "access_point":
      return <Wifi className={cls} aria-hidden />;
    case "server":
    case "domain_controller":
      return <Server className={cls} aria-hidden />;
    case "nas":
      return <HardDrive className={cls} aria-hidden />;
    case "workstation":
    case "computer":
      return <Monitor className={cls} aria-hidden />;
    case "printer":
      return <Printer className={cls} aria-hidden />;
    case "camera":
      return <Camera className={cls} aria-hidden />;
    default:
      return <HelpCircle className={cls} aria-hidden />;
  }
}

function strokeFor(confidence: TopologyConnection["confidence"]): { dash: string; width: number } {
  if (confidence === "confirmed") return { dash: "", width: 2.4 };
  if (confidence === "strong") return { dash: "", width: 1.7 };
  return { dash: "4 4", width: 1.5 };
}

export function TopologyPreview({
  snapshot,
  names,
  types,
  physical,
  diagnostics,
}: TopologyPreviewProps) {
  const [showEndpoints, setShowEndpoints] = useState(true);
  const [zoom, setZoom] = useState(1);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const unknownNodes: UnresolvedNode[] = snapshot.unknownNodes ?? [];

  const layout = useMemo(
    () => layoutTopology({ snapshot, names, types, physical, showEndpoints }),
    [snapshot, names, types, physical, showEndpoints],
  );

  const viewW = Math.max(layout.width / zoom, 1);
  const viewH = Math.max(layout.height / zoom, 1);
  const viewX = (layout.width - viewW) / 2;
  const viewY = (layout.height - viewH) / 2;

  const selected = layout.edges.find((edge) => edge.id === selectedId)?.connection ?? null;

  const fit = () => setZoom(1);
  const reset = () => {
    setZoom(1);
    setShowEndpoints(true);
    setSelectedId(null);
  };

  if (layout.nodes.length === 0) {
    return (
      <p className="text-[13px] leading-relaxed text-text-secondary">
        Nothing to draw yet. Proven neighbour relationships appear here after a topology run.
      </p>
    );
  }

  return (
    <div className="topo-preview">
      <div className="mb-2 flex flex-wrap items-center gap-1.5">
        <Tooltip content="Fit the diagram to the available space.">
          <IconButton label="Fit" size="sm" onClick={fit}>
            <Focus className="h-3.5 w-3.5" />
          </IconButton>
        </Tooltip>
        <Tooltip content="Zoom in.">
          <IconButton
            label="Zoom in"
            size="sm"
            onClick={() => setZoom((value) => Math.min(2, value + 0.2))}
          >
            <ZoomIn className="h-3.5 w-3.5" />
          </IconButton>
        </Tooltip>
        <Tooltip content="Zoom out.">
          <IconButton
            label="Zoom out"
            size="sm"
            onClick={() => setZoom((value) => Math.max(0.5, value - 0.2))}
          >
            <ZoomOut className="h-3.5 w-3.5" />
          </IconButton>
        </Tooltip>
        <Tooltip content="Show or hide workstations, printers, cameras and other endpoints.">
          <button
            type="button"
            aria-pressed={!showEndpoints}
            onClick={() => setShowEndpoints((value) => !value)}
            className="btn btn-ghost btn-sm"
          >
            {showEndpoints ? "Hide endpoints" : "Show endpoints"}
          </button>
        </Tooltip>
        <Tooltip content="Reset zoom, pan and the endpoint layer.">
          <IconButton label="Reset layout" size="sm" onClick={reset}>
            <RotateCcw className="h-3.5 w-3.5" />
          </IconButton>
        </Tooltip>
      </div>

      <div className="topo-preview-canvas">
        <svg
          role="img"
          aria-label="Topology preview"
          viewBox={`${viewX} ${viewY} ${viewW} ${viewH}`}
          className="h-[400px] w-full"
        >
          {layout.edges.map((edge) => {
            const from = layout.nodes.find((node) => node.id === edge.from);
            const to = layout.nodes.find((node) => node.id === edge.to);
            if (!from || !to) return null;
            const x1 = from.x + PREVIEW_NODE_WIDTH / 2;
            const y1 = from.y + PREVIEW_NODE_HEIGHT;
            const x2 = to.x + PREVIEW_NODE_WIDTH / 2;
            const y2 = to.y;
            const midY = (y1 + y2) / 2;
            const stroke = strokeFor(edge.connection.confidence);
            const active = selectedId === edge.id;
            return (
              <g key={edge.id}>
                <path
                  d={`M ${x1} ${y1} C ${x1} ${midY}, ${x2} ${midY}, ${x2} ${y2}`}
                  fill="none"
                  stroke="transparent"
                  strokeWidth={14}
                  className="topo-edge-hit cursor-pointer"
                  role="button"
                  tabIndex={0}
                  aria-label={connectionDetailLines(edge.connection, names, unknownNodes).join(" · ")}
                  aria-pressed={active}
                  onClick={() => setSelectedId(edge.id)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setSelectedId(edge.id);
                    }
                  }}
                />
                <path
                  d={`M ${x1} ${y1} C ${x1} ${midY}, ${x2} ${midY}, ${x2} ${y2}`}
                  fill="none"
                  stroke={active ? "var(--accent)" : "var(--border-strong)"}
                  strokeWidth={stroke.width}
                  strokeDasharray={stroke.dash}
                  className="pointer-events-none"
                />
              </g>
            );
          })}
          {layout.nodes.map((node) => (
            <g key={node.id} transform={`translate(${node.x} ${node.y})`}>
              {node.kind === "internet" ? (
                <ellipse
                  cx={PREVIEW_NODE_WIDTH / 2}
                  cy={PREVIEW_NODE_HEIGHT / 2}
                  rx={PREVIEW_NODE_WIDTH / 2 - 4}
                  ry={PREVIEW_NODE_HEIGHT / 2 - 2}
                  className="topo-node-internet"
                />
              ) : (
                <rect
                  width={PREVIEW_NODE_WIDTH}
                  height={PREVIEW_NODE_HEIGHT}
                  rx={10}
                  className="topo-node"
                />
              )}
              <foreignObject width={PREVIEW_NODE_WIDTH} height={PREVIEW_NODE_HEIGHT}>
                <div className="flex h-full items-center gap-2 px-2.5">
                  <span className="text-accent-text">
                    <KindIcon kind={node.kind} />
                  </span>
                  <span className="min-w-0">
                    <span className="block truncate text-[12px] font-medium leading-tight text-text">
                      {node.label}
                    </span>
                    <span className="block truncate text-[10px] leading-tight text-text-muted">
                      {roleCaption(node)}
                    </span>
                  </span>
                </div>
              </foreignObject>
            </g>
          ))}
        </svg>
      </div>

      {selected ? (
        <ConnectionCard
          connection={selected}
          names={names}
          unknownNodes={unknownNodes}
          why={whyForConnection(selected, diagnostics)}
          onDismiss={() => setSelectedId(null)}
        />
      ) : (
        <p className="mt-2 text-xs text-text-muted">
          Select a connection — click, or tab to it and press Enter — for ports, protocol,
          confidence, speed, VLAN, PoE and evidence.
        </p>
      )}
    </div>
  );
}

function ConnectionCard({
  connection,
  names,
  unknownNodes,
  why,
  onDismiss,
}: {
  connection: TopologyConnection;
  names: DeviceNameLookup;
  unknownNodes: UnresolvedNode[];
  why: string | null;
  onDismiss: () => void;
}) {
  const lines = connectionDetailLines(connection, names, unknownNodes);
  const speed = speedLabel(connection.speedMbps);
  const vlan = vlanLabel(connection);
  return (
    <div className="mt-2 rounded-md border border-border bg-surface-raised px-3 py-2 text-xs leading-relaxed text-text-secondary">
      <div className="flex items-start justify-between gap-2">
        <p className="font-medium text-text">{lines[0]}</p>
        <button type="button" className="btn btn-ghost btn-sm" onClick={onDismiss}>
          Close
        </button>
      </div>
      <p className="mt-1 flex flex-wrap items-center gap-1.5">
        <Tooltip content={CONFIDENCE_HINT[connection.confidence]}>
          <span className="rounded-full bg-surface-sunken px-1.5 py-0.5 text-[11px] text-text">
            {confidenceLabel(connection.confidence)}
          </span>
        </Tooltip>
        <span>{protocolLabel(connection.protocol)}</span>
        {speed ? <span>· {speed}</span> : null}
        {vlan ? <span>· {vlan}</span> : null}
        {connection.poe?.enabled ? (
          <span>· PoE{connection.poe.watts != null ? ` ${connection.poe.watts} W` : ""}</span>
        ) : null}
      </p>
      {connection.evidence[0] ? <p className="mt-1 text-text-muted">{connection.evidence[0]}</p> : null}
      {connection.evidence.length > 0 || why ? (
        <details className="mt-1">
          <summary className="cursor-pointer text-text">Why this connection?</summary>
          {why ? <p className="mt-1 text-text-secondary">{why}</p> : null}
          <ul className="mt-1 space-y-1 text-text-muted">
            {connection.evidence.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        </details>
      ) : null}
    </div>
  );
}
