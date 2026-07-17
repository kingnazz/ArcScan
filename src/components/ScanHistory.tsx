import { Clock, FolderOpen, Trash2 } from "lucide-react";
import type { ScanSummary } from "../types";
import { formatDateTime, formatDuration } from "../lib/format";

interface ScanHistoryProps {
  scans: ScanSummary[];
  activeId: number | null;
  onOpen: (id: number) => void;
  onDelete: (id: number) => void;
}

export function ScanHistory({ scans, activeId, onOpen, onDelete }: ScanHistoryProps) {
  return (
    <div className="panel flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-line px-4 py-3 text-sm font-medium text-fg">
        <Clock className="h-4 w-4 text-brand-600 dark:text-brand-300" />
        Scan history
        <span className="ml-auto text-xs text-faint">{scans.length} saved</span>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-2">
        {scans.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-1 py-16 text-center text-muted">
            <p className="text-sm">No saved scans yet.</p>
            <p className="text-xs text-faint">Completed scans are saved here automatically.</p>
          </div>
        ) : (
          <ul className="space-y-1.5">
            {scans.map((s) => (
              <li key={s.id}>
                <div
                  className={`group flex items-center gap-3 rounded-lg border px-3 py-2.5 transition-colors ${
                    activeId === s.id
                      ? "border-brand-500/40 bg-brand-500/10"
                      : "border-line bg-surface hover:border-brand-400/40 hover:bg-surface2"
                  }`}
                >
                  <button className="flex min-w-0 flex-1 items-center gap-3 text-left" onClick={() => onOpen(s.id)}>
                    <div className="min-w-0 flex-1">
                      <div className="truncate font-mono text-sm text-fg">{s.target}</div>
                      <div className="mt-0.5 text-xs text-faint">{formatDateTime(s.created_at)}</div>
                    </div>
                    <div className="shrink-0 text-right">
                      <div className="text-sm font-semibold tabular-nums text-brand-600 dark:text-brand-300">
                        {s.host_count}
                      </div>
                      <div className="text-[11px] text-faint">
                        {s.host_count === 1 ? "host" : "hosts"} · {formatDuration(s.duration_ms)}
                      </div>
                    </div>
                  </button>
                  <div className="flex shrink-0 items-center gap-0.5">
                    <button className="btn-icon" title="Open scan" onClick={() => onOpen(s.id)}>
                      <FolderOpen className="h-4 w-4" />
                    </button>
                    <button
                      className="btn-icon hover:text-red-500 dark:hover:text-red-300"
                      title="Delete scan"
                      onClick={() => onDelete(s.id)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
