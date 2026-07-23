import { FolderOpen, Trash2 } from "lucide-react";
import type { ScanSummary } from "../types";
import { formatDateTime, formatDuration } from "../lib/format";

interface ScanHistoryProps {
  scans: ScanSummary[];
  activeId: number | null;
  onOpen: (id: number) => void;
  onDelete: (id: number) => void;
}

// History as a dense grid, matching the results table.
export function ScanHistory({ scans, activeId, onOpen, onDelete }: ScanHistoryProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <div className="min-h-0 flex-1 overflow-auto">
        {scans.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-1 py-16 text-center text-muted">
            <p className="text-sm">No saved scans yet.</p>
            <p className="text-xs text-faint">Completed scans are saved here automatically.</p>
          </div>
        ) : (
          <table className="grid-table">
            <thead>
              <tr>
                <th>Target</th>
                <th>Date</th>
                <th className="text-right">Hosts</th>
                <th className="text-right">Scanned</th>
                <th className="text-right">Duration</th>
                <th className="text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {scans.map((s) => (
                <tr
                  key={s.id}
                  className={`group cursor-pointer ${activeId === s.id ? "!bg-brand-500/15" : ""}`}
                  onClick={() => onOpen(s.id)}
                >
                  <td className="font-mono text-fg">{s.target}</td>
                  <td className="text-muted">{formatDateTime(s.created_at)}</td>
                  <td className="text-right font-semibold tabular-nums text-brand-700 dark:text-brand-300">
                    {s.host_count}
                  </td>
                  <td className="text-right tabular-nums text-muted">{s.scanned}</td>
                  <td className="text-right tabular-nums text-muted">{formatDuration(s.duration_ms)}</td>
                  <td className="!py-0">
                    <div className="flex items-center justify-end gap-0 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                      <button
                        className="btn-icon"
                        title="Open scan"
                        onClick={(e) => {
                          e.stopPropagation();
                          onOpen(s.id);
                        }}
                      >
                        <FolderOpen className="h-3.5 w-3.5" />
                      </button>
                      <button
                        className="btn-icon hover:text-red-500 dark:hover:text-red-400"
                        title="Delete scan"
                        onClick={(e) => {
                          e.stopPropagation();
                          onDelete(s.id);
                        }}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
