import { Clock, History, Trash2 } from "lucide-react";
import type { ScanSummary } from "../types";
import { formatTime } from "../lib/format";

interface ScanHistoryProps {
  scans: ScanSummary[];
  activeId: number | null;
  onSelect: (scan: ScanSummary) => void;
  onDelete: (id: number) => void;
}

export function ScanHistory({ scans, activeId, onSelect, onDelete }: ScanHistoryProps) {
  return (
    <div className="flex flex-col min-h-0 flex-1">
      <div className="flex items-center gap-2 px-3 py-2 text-xs font-semibold uppercase tracking-wide text-slate-400">
        <History className="h-3.5 w-3.5" />
        Scan History
      </div>
      <div className="overflow-auto flex-1 px-2 space-y-1">
        {scans.length === 0 && (
          <p className="px-2 py-6 text-xs text-slate-600 text-center">
            No saved scans yet. Run a scan to build history.
          </p>
        )}
        {scans.map((scan) => (
          <button
            key={scan.id}
            onClick={() => onSelect(scan)}
            className={`group w-full text-left rounded-lg px-2.5 py-2 transition-colors ${
              activeId === scan.id ? "bg-accent/15 ring-1 ring-accent/40" : "hover:bg-base-700/60"
            }`}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="font-mono text-xs text-slate-200 truncate">{scan.target}</span>
              <span
                role="button"
                tabIndex={0}
                title="Delete scan"
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(scan.id);
                }}
                className="icon-btn h-6 w-6 opacity-0 group-hover:opacity-100"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </span>
            </div>
            <div className="mt-1 flex items-center gap-2 text-[11px] text-slate-500">
              <Clock className="h-3 w-3" />
              {formatTime(scan.finishedAt)}
              <span className="text-ok tabular-nums ml-auto">{scan.hostsUp} up</span>
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}
