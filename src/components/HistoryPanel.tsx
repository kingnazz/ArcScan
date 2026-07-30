// Scan history as an operational timeline.
//
// Each entry answers the question an operator actually has: what was scanned,
// when, and what was different. The change counts come from the scan row itself,
// so listing a thousand scans is still one query.

import { Download, FolderOpen, GitCompare, Trash2 } from "lucide-react";
import { Badge, EmptyState, IconButton } from "../ui/primitives";
import { formatCount, formatDateTime, formatDuration } from "../lib/format";
import { profileName } from "../lib/profiles";
import type { ScanSummary } from "../types";

export interface HistoryPanelProps {
  scans: ScanSummary[];
  activeId: number | null;
  onOpen: (id: number) => void;
  onCompare: (id: number) => void;
  onDelete: (id: number) => void;
  onExport: (id: number) => void;
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
        description="Every completed scan is saved here automatically, along with what changed since the previous scan of the same target."
      />
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <ul className="divide-y divide-border">
        {scans.map((scan) => {
          const active = scan.id === activeId;
          const changes = scan.new_count + scan.missing_count + scan.changed_count;
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
                  {scan.status === "cancelled" ? (
                    <Badge tone="warning" title="Stopped before it finished">
                      Stopped
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
                </p>
              </button>

              <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity duration-fast focus-within:opacity-100 group-hover:opacity-100">
                <IconButton label="Open this scan" size="sm" onClick={() => onOpen(scan.id)}>
                  <FolderOpen className="h-3.5 w-3.5" />
                </IconButton>
                <IconButton
                  label={
                    scan.baseline_scan_id == null
                      ? "No earlier compatible scan to compare with"
                      : "Compare with the previous scan"
                  }
                  size="sm"
                  disabled={scan.baseline_scan_id == null}
                  onClick={() => onCompare(scan.id)}
                >
                  <GitCompare className="h-3.5 w-3.5" />
                </IconButton>
                <IconButton label="Export this scan" size="sm" onClick={() => onExport(scan.id)}>
                  <Download className="h-3.5 w-3.5" />
                </IconButton>
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
