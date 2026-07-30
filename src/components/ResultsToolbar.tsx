// The toolbar above the results table: filtering, the change summary, and export.

import { forwardRef, useRef, useState } from "react";
import { Download, Filter, Search, Sparkles, X } from "lucide-react";
import { Badge, Button, Field, IconButton } from "../ui/primitives";
import { Popover } from "../ui/Popover";
import { formatCount } from "../lib/format";
import type { ExportFormat, ScanComparison } from "../types";
import type { TableFilter } from "../lib/table";

export interface ResultsToolbarProps {
  filter: TableFilter;
  onFilterChange: (patch: Partial<TableFilter>) => void;
  shown: number;
  total: number;
  comparison: ScanComparison | null;
  onExport: (format: ExportFormat) => void;
  onViewChanges: () => void;
  canExport: boolean;
}

export const ResultsToolbar = forwardRef<HTMLInputElement, ResultsToolbarProps>(
  function ResultsToolbar(
    { filter, onFilterChange, shown, total, comparison, onExport, onViewChanges, canExport },
    filterRef,
  ) {
    const exportButton = useRef<HTMLButtonElement>(null);
    const [exportOpen, setExportOpen] = useState(false);

    const newCount = comparison?.added.filter((d) => d.kind === "new").length ?? 0;
    const returnedCount = comparison?.added.filter((d) => d.kind === "returned").length ?? 0;
    const changedCount = comparison?.changed.length ?? 0;
    const missingCount = comparison?.removed.length ?? 0;
    const anyChanges = newCount + returnedCount + changedCount + missingCount > 0;

    return (
      <div className="flex flex-wrap items-center gap-2 border-b border-border bg-surface px-3 py-1.5">
        <div className="relative w-56 shrink-0">
          <Search
            className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-text-muted"
            aria-hidden
          />
          <Field
            ref={filterRef}
            className="pl-7 pr-7"
            placeholder="Filter devices"
            aria-label="Filter devices by name, address, vendor or service"
            value={filter.query}
            onChange={(event) => onFilterChange({ query: event.target.value })}
          />
          {filter.query ? (
            <button
              type="button"
              aria-label="Clear the filter"
              onClick={() => onFilterChange({ query: "" })}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-text-muted hover:text-text"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          ) : null}
        </div>

        <span className="shrink-0 text-xs text-text-muted">
          {shown === total ? (
            <>
              <span className="font-medium text-text-secondary">{formatCount(total)}</span>{" "}
              {total === 1 ? "device" : "devices"}
            </>
          ) : (
            <>
              <span className="font-medium text-text-secondary">{formatCount(shown)}</span> of{" "}
              {formatCount(total)}
            </>
          )}
        </span>

        <Button
          size="sm"
          variant={filter.savedOnly ? "primary" : "ghost"}
          icon={<Filter className="h-3.5 w-3.5" />}
          aria-pressed={filter.savedOnly}
          onClick={() => onFilterChange({ savedOnly: !filter.savedOnly })}
          title="Show only devices you have marked known, trusted or watched"
        >
          Labelled
        </Button>

        {anyChanges ? (
          <Button
            size="sm"
            variant={filter.changesOnly ? "primary" : "ghost"}
            icon={<Sparkles className="h-3.5 w-3.5" />}
            aria-pressed={filter.changesOnly}
            onClick={() => onFilterChange({ changesOnly: !filter.changesOnly })}
            title="Show only devices that arrived or changed since the previous scan"
          >
            Changes only
          </Button>
        ) : null}

        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          {anyChanges ? (
            <button
              type="button"
              onClick={onViewChanges}
              className="flex items-center gap-1.5 rounded-md px-1.5 py-1 transition-colors duration-fast hover:bg-surface-hover"
              title="Open the full comparison"
            >
              {newCount > 0 ? <Badge tone="new">{newCount} new</Badge> : null}
              {returnedCount > 0 ? <Badge tone="accent">{returnedCount} back</Badge> : null}
              {changedCount > 0 ? <Badge tone="changed">{changedCount} changed</Badge> : null}
              {missingCount > 0 ? <Badge tone="missing">{missingCount} missing</Badge> : null}
            </button>
          ) : comparison && comparison.baseline_scan_id != null ? (
            <Badge>No changes</Badge>
          ) : null}

          <div className="relative">
            <Button
              ref={exportButton}
              size="sm"
              icon={<Download className="h-3.5 w-3.5" />}
              disabled={!canExport}
              aria-expanded={exportOpen}
              aria-haspopup="dialog"
              onClick={() => setExportOpen((v) => !v)}
              title="Export the filtered devices (Ctrl or Cmd + E)"
            >
              Export
            </Button>
            <Popover
              open={exportOpen}
              onClose={() => setExportOpen(false)}
              anchor={exportButton}
              label="Export format"
              className="w-44 p-1"
            >
              {(
                [
                  ["csv", "CSV spreadsheet"],
                  ["json", "JSON"],
                  ["xml", "XML"],
                ] as Array<[ExportFormat, string]>
              ).map(([format, label]) => (
                <button
                  key={format}
                  type="button"
                  className="w-full rounded px-2.5 py-1.5 text-left text-[13px] text-text transition-colors duration-fast hover:bg-surface-hover"
                  onClick={() => {
                    setExportOpen(false);
                    onExport(format);
                  }}
                >
                  {label}
                </button>
              ))}
              <p className="px-2.5 pb-1 pt-1.5 text-xs leading-relaxed text-text-muted">
                Exports the {formatCount(shown)} {shown === 1 ? "device" : "devices"} currently
                shown.
              </p>
            </Popover>
          </div>

          <IconButton label="Clear the filter" size="sm" onClick={() => onFilterChange({ query: "", savedOnly: false, changesOnly: false })}>
            <X className="h-3.5 w-3.5" />
          </IconButton>
        </div>
      </div>
    );
  },
);
