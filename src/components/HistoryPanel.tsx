// Scan history as an operational timeline.
//
// It answers "what happened during previous scans?" — the record behind the
// Inventory (what exists) and Changes (what moved).
//
// Each entry answers the question an operator actually has: what was scanned,
// when, and what was different. The change counts come from the scan row itself,
// so listing a thousand scans is still one query.

import { GitCompare, Trash2 } from "lucide-react";
import { Badge, Button, EmptyState, IconButton } from "../ui/primitives";
import { ExportMenu, ViewHeader } from "../ui/patterns";
import { formatCount, formatDateTime, formatDuration } from "../lib/format";
import { DISCOVERY_QUALITY_HINT, discoverySummaryLine } from "../lib/discovery";
import { profileName } from "../lib/profiles";
import type { ExportFormat, ScanSummary } from "../types";

export interface HistoryPanelProps {
  scans: ScanSummary[];
  activeId: number | null;
  onOpen: (id: number) => void;
  onCompare: (id: number) => void;
  onDelete: (id: number) => void;
  onExport: (id: number, format: ExportFormat) => void;
  onStartScan?: () => void;
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
  onStartScan,
}: HistoryPanelProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title="History"
        summary={scans.length > 0 ? `${formatCount(scans.length)} ${scans.length === 1 ? "scan" : "scans"}` : undefined}
        purpose="Every scan that ran, newest first, and what each one found. Open a scan to see its devices, or compare it with the one before."
      />
      {scans.length === 0 ? (
        <EmptyState
          title="No saved scans yet"
          description="Every scan is saved here automatically, along with what changed since the previous completed scan of the same target."
          action={onStartScan ? <Button onClick={onStartScan}>Go to Scan</Button> : undefined}
        />
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <ul className="divide-y divide-border">
            {scans.map((scan) => {
              const active = scan.id === activeId;
              const changes = scan.new_count + scan.missing_count + scan.changed_count;
              const noCompare = compareUnavailableReason(scan);
              return (
                <li
                  key={scan.id}
                  className={`flex items-center gap-3 px-4 py-2.5 transition-colors duration-fast hover:bg-surface-hover ${
                    active ? "bg-accent-subtle shadow-[inset_2px_0_0_var(--accent)]" : ""
                  }`}
                >
                  <button
                    type="button"
                    onClick={() => onOpen(scan.id)}
                    className="flex min-w-0 flex-1 items-start gap-4 rounded text-left"
                    aria-current={active ? "true" : undefined}
                    title="Open this scan"
                  >
                    <span className="w-36 shrink-0 pt-px text-xs text-text-secondary" title={scan.created_at}>
                      {formatDateTime(scan.created_at)}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                        <span className="mono text-[13px] font-semibold text-text">{scan.target}</span>
                        {scan.scope_name ? (
                          <span className="text-xs text-text-secondary" title={`Network: ${scan.scope_name}`}>
                            {scan.scope_name}
                          </span>
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
                          <span className="text-xs text-text-muted">No changes</span>
                        ) : null}
                      </span>
                      <span className="mt-1 block text-xs text-text-muted">
                        <span className="text-text-secondary">{formatCount(scan.host_count)}</span>{" "}
                        {scan.host_count === 1 ? "device" : "devices"}
                        {" · "}
                        {scan.status === "cancelled"
                          ? `${formatCount(scan.probed)} of ${formatCount(scan.scanned)} addresses checked`
                          : `${formatCount(scan.scanned)} addresses`}
                        {" · "}
                        {formatDuration(scan.duration_ms)}
                        {" · "}
                        {profileName(scan.profile)}
                        {/* How well the discovery pass went, and either what it
                            heard or the one thing that limited it. Only shown
                            once a scan has recorded discovery at all, so history
                            from before v1.8.2 stays exactly as it was. */}
                        {scan.discovery_quality || scan.discovery_mode !== "none" ? (
                          <>
                            {" · "}
                            <span title={DISCOVERY_QUALITY_HINT[scan.discovery_quality ?? "skipped"]}>
                              Discovery: {discoverySummaryLine(scan)}
                            </span>
                          </>
                        ) : null}
                      </span>
                    </span>
                  </button>

                  {/* Always visible, never only on hover: a control that appears
                      under the pointer is one nobody finds by keyboard or by
                      looking. Quiet, so twenty rows of them do not shout. */}
                  <div className="flex shrink-0 items-center gap-0.5">
                    <IconButton
                      label={noCompare ?? "Compare with the previous scan"}
                      size="sm"
                      disabled={noCompare != null}
                      onClick={() => onCompare(scan.id)}
                    >
                      <GitCompare className="h-3.5 w-3.5" />
                    </IconButton>
                    <ExportMenu
                      compact
                      label="Export this scan"
                      onExport={(format) => onExport(scan.id, format)}
                      scope="Exports this scan's own saved devices."
                    />
                    <IconButton
                      label="Delete this scan"
                      size="sm"
                      className="hover:!text-danger"
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
      )}
    </div>
  );
}
