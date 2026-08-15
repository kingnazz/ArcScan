// Scan history as an operational timeline.
//
// Each entry answers the question an operator actually has: what was scanned,
// when, and what was different. The change counts come from the scan row itself,
// so listing a thousand scans is still one query.

import { useRef, useState } from "react";
import { Download, FolderOpen, GitCompare, Trash2 } from "lucide-react";
import { Badge, EmptyState, IconButton } from "../ui/primitives";
import { Popover } from "../ui/Popover";
import { formatCount, formatDateTime, formatDuration } from "../lib/format";
import { discoveryModeLabel } from "../lib/discovery";
import { profileName } from "../lib/profiles";
import type { ExportFormat, ScanSummary } from "../types";

export interface HistoryPanelProps {
  scans: ScanSummary[];
  activeId: number | null;
  onOpen: (id: number) => void;
  onCompare: (id: number) => void;
  onDelete: (id: number) => void;
  onExport: (id: number, format: ExportFormat) => void;
}

/** Why a scan cannot be compared, for the disabled compare button's tooltip. */
export function compareUnavailableReason(scan: ScanSummary): string | null {
  if (scan.status === "cancelled") {
    return "This scan was stopped early, so it cannot be compared reliably";
  }
  if (scan.baseline_scan_id == null) {
    return "No earlier completed scan checked the same target and ports";
  }
  return null;
}

export function HistoryPanel({
  scans,
  activeId,
  onOpen,
  onCompare,
  onDelete,
  onExport,
}: HistoryPanelProps) {
  if (scans.length === 0) {
    return (
      <EmptyState
        title="No saved scans yet"
        description="Every scan is saved here automatically, along with what changed since the previous completed scan of the same target."
      />
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <ul className="divide-y divide-border">
        {scans.map((scan) => {
          const active = scan.id === activeId;
          const changes = scan.new_count + scan.missing_count + scan.changed_count;
          const noCompare = compareUnavailableReason(scan);
          return (
            <li
              key={scan.id}
              className={`group flex items-center gap-3 px-3 py-2.5 transition-colors duration-fast hover:bg-surface-hover ${
                active ? "bg-accent-subtle" : ""
              }`}
            >
              <button
                type="button"
                onClick={() => onOpen(scan.id)}
                className="min-w-0 flex-1 text-left"
                aria-current={active ? "true" : undefined}
              >
                <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                  <span className="mono text-[13px] font-medium text-text">{scan.target}</span>
                  <Badge>{profileName(scan.profile)}</Badge>
                  {scan.scope_name ? (
                    <Badge title={`Network: ${scan.scope_name}`}>{scan.scope_name}</Badge>
                  ) : null}
                  {scan.status === "cancelled" ? (
                    <Badge
                      tone="warning"
                      title="Stopped before every address was checked, so changes are unavailable"
                    >
                      Partial scan
                    </Badge>
                  ) : null}
                  {scan.new_count > 0 ? <Badge tone="new">{scan.new_count} new</Badge> : null}
                  {scan.changed_count > 0 ? (
                    <Badge tone="changed">{scan.changed_count} changed</Badge>
                  ) : null}
                  {scan.missing_count > 0 ? (
                    <Badge tone="missing">{scan.missing_count} missing</Badge>
                  ) : null}
                  {changes === 0 && scan.baseline_scan_id != null ? (
                    <Badge>No changes</Badge>
                  ) : null}
                </div>
                <p className="mt-1 text-xs text-text-muted">
                  <span title={scan.created_at}>{formatDateTime(scan.created_at)}</span>
                  {" · "}
                  <span className="text-text-secondary">{formatCount(scan.host_count)}</span>{" "}
                  {scan.host_count === 1 ? "device" : "devices"}
                  {" · "}
                  {scan.status === "cancelled"
                    ? `${formatCount(scan.probed)} of ${formatCount(scan.scanned)} addresses checked`
                    : `${formatCount(scan.scanned)} addresses`}
                  {" · "}
                  {formatDuration(scan.duration_ms)}
                  {/* What the discovery pass managed, so a scan that heard
                      nothing reads as "no discovery" rather than as a network
                      with nothing on it. Only shown once a scan has recorded
                      one, so history from before this version stays unchanged. */}
                  {scan.discovery_mode && scan.discovery_mode !== "none" ? (
                    <>
                      {" · "}
                      {discoveryModeLabel(scan.discovery_mode)}
                    </>
                  ) : null}
                </p>
              </button>

              <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity duration-fast focus-within:opacity-100 group-hover:opacity-100">
                <IconButton label="Open this scan" size="sm" onClick={() => onOpen(scan.id)}>
                  <FolderOpen className="h-3.5 w-3.5" />
                </IconButton>
                <IconButton
                  label={noCompare ?? "Compare with the previous scan"}
                  size="sm"
                  disabled={noCompare != null}
                  onClick={() => onCompare(scan.id)}
                >
                  <GitCompare className="h-3.5 w-3.5" />
                </IconButton>
                <ExportMenu scanId={scan.id} onExport={onExport} />
                <IconButton
                  label="Delete this scan"
                  size="sm"
                  className="hover:text-danger"
                  onClick={() => onDelete(scan.id)}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </IconButton>
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

const EXPORT_FORMATS: ExportFormat[] = ["csv", "json", "xml"];

/** A per-row export button with a small format picker. The export always uses
 * the scan's own saved rows, never the currently displayed table. */
function ExportMenu({
  scanId,
  onExport,
}: {
  scanId: number;
  onExport: (id: number, format: ExportFormat) => void;
}) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);

  return (
    <div className="relative">
      <IconButton
        ref={anchor}
        label="Export this scan"
        size="sm"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <Download className="h-3.5 w-3.5" />
      </IconButton>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        align="end"
        label="Export format"
        className="w-28 p-1"
      >
        <ul role="menu" aria-label="Export format">
          {EXPORT_FORMATS.map((format) => (
            <li key={format}>
              <button
                type="button"
                role="menuitem"
                className="w-full rounded-md px-2.5 py-1.5 text-left text-[13px] text-text transition-colors duration-fast hover:bg-surface-hover"
                onClick={() => {
                  setOpen(false);
                  onExport(scanId, format);
                }}
              >
                {format.toUpperCase()}
              </button>
            </li>
          ))}
        </ul>
      </Popover>
    </div>
  );
}
