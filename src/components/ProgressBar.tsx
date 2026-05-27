import type { ScanProgress } from "../types";

interface ProgressBarProps {
  progress: ScanProgress;
  active: boolean;
}

export function ProgressBar({ progress, active }: ProgressBarProps) {
  const pct = progress.total > 0 ? Math.min(100, (progress.scanned / progress.total) * 100) : 0;
  if (!active && progress.total === 0) return null;

  return (
    <div className="flex items-center gap-3">
      <div className="h-1.5 flex-1 rounded-full bg-base-700 overflow-hidden">
        <div
          className={`h-full rounded-full bg-accent transition-[width] duration-150 ${
            active ? "shadow-glow" : ""
          }`}
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="text-xs text-slate-400 tabular-nums whitespace-nowrap">
        {progress.scanned.toLocaleString()} / {progress.total.toLocaleString()} ·{" "}
        <span className="text-ok">{progress.found} up</span>
      </span>
    </div>
  );
}
