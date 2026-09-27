// The toolbar above the scan results.
//
// Left: how the scan is being looked at — its device list, its comparison with
// the previous scan, or topology — and, for the device list, the filters.
// Right: what to do with a finished scan — export it, or hand it to ArcAtlas.
// The comparison used to be reachable only through an uppercase badge that did
// not look like a button; it is a named view of the scan now.

import { forwardRef } from "react";
import { Filter, Send, Sparkles } from "lucide-react";
import { Button } from "../ui/primitives";
import { ExportMenu, SearchField } from "../ui/patterns";
import { TOPOLOGY_HINT } from "../lib/topology";
import { formatCount } from "../lib/format";
import type { ExportFormat, ScanComparison } from "../types";
import type { TableFilter } from "../lib/table";

export type ScanTab = "devices" | "comparison" | "topology";

export interface ResultsToolbarProps {
  tab: ScanTab;
  onTabChange: (tab: ScanTab) => void;
  filter: TableFilter;
  onFilterChange: (patch: Partial<TableFilter>) => void;
  shown: number;
  total: number;
  comparison: ScanComparison | null;
  onExport: (format: ExportFormat) => void;
  canExport: boolean;
  /**
   * The ArcAtlas handoff, offered only once a completed scan has been saved:
   * it is the end of the workflow, not a feature to advertise before it.
   */
  handoff?: { onSend: () => void; enabled: boolean; title: string } | null;
}

/** New, returned, changed and missing, counted once. */
export function comparisonCount(comparison: ScanComparison | null): number {
  if (!comparison || comparison.baseline_scan_id == null) return 0;
  return comparison.added.length + comparison.changed.length + comparison.removed.length;
}

export const ResultsToolbar = forwardRef<HTMLInputElement, ResultsToolbarProps>(
  function ResultsToolbar(
    { tab, onTabChange, filter, onFilterChange, shown, total, comparison, onExport, canExport, handoff },
    filterRef,
  ) {
    const changes = comparisonCount(comparison);
    const anyFilter = filter.query.length > 0 || filter.savedOnly || filter.changesOnly;

    return (
      <div className="toolbar">
        <div className="segmented" role="group" aria-label="Show">
          <button
            type="button"
            className="segmented-item"
            aria-pressed={tab === "devices"}
            onClick={() => onTabChange("devices")}
          >
            Devices
            <span className="text-text-muted">{formatCount(total)}</span>
          </button>
          <button
            type="button"
            className="segmented-item"
            aria-pressed={tab === "comparison"}
            onClick={() => onTabChange("comparison")}
            title="This scan compared with the previous completed scan of the same target"
          >
            Comparison
            {changes > 0 ? <span className="nav-badge nav-badge-attention">{changes}</span> : null}
          </button>
          <button
            type="button"
            className="segmented-item"
            aria-pressed={tab === "topology"}
            onClick={() => onTabChange("topology")}
            title={TOPOLOGY_HINT}
          >
            Topology
          </button>
        </div>

        {tab === "devices" ? (
          <>
            <SearchField
              ref={filterRef}
              value={filter.query}
              onChange={(query) => onFilterChange({ query })}
              placeholder="Filter devices"
              label="Filter devices by name, address, vendor or service"
            />
            <Button
              size="sm"
              variant="ghost"
              icon={<Filter className="h-3.5 w-3.5" />}
              aria-pressed={filter.savedOnly}
              onClick={() => onFilterChange({ savedOnly: !filter.savedOnly })}
              title="Show only devices you have marked Known, Trusted or Watched"
            >
              Classified
            </Button>
            {changes > 0 ? (
              <Button
                size="sm"
                variant="ghost"
                icon={<Sparkles className="h-3.5 w-3.5" />}
                aria-pressed={filter.changesOnly}
                onClick={() => onFilterChange({ changesOnly: !filter.changesOnly })}
                title="Show only devices that arrived or changed since the previous scan"
              >
                New or changed
              </Button>
            ) : null}
            {anyFilter ? (
              <span className="flex shrink-0 items-center gap-2 text-xs text-text-muted">
                <span>
                  <span className="font-medium text-text-secondary">{formatCount(shown)}</span> of{" "}
                  {formatCount(total)}
                </span>
                <button
                  type="button"
                  className="link-btn"
                  onClick={() => onFilterChange({ query: "", savedOnly: false, changesOnly: false })}
                >
                  Clear filters
                </button>
              </span>
            ) : null}
          </>
        ) : null}

        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          <ExportMenu
            onExport={onExport}
            disabled={!canExport}
            title="Export the filtered devices (Ctrl or Cmd + E)"
            scope={`Exports the ${formatCount(shown)} ${shown === 1 ? "device" : "devices"} currently shown.`}
          />
          {handoff ? (
            <Button
              size="sm"
              icon={<Send className="h-3.5 w-3.5" />}
              disabled={!handoff.enabled}
              onClick={handoff.onSend}
              title={handoff.title}
            >
              Send to ArcAtlas
            </Button>
          ) : null}
        </div>
      </div>
    );
  },
);
