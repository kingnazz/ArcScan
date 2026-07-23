import { Wifi, WifiOff } from "lucide-react";
import type { DashboardStats, ScanProgress } from "../types";

interface StatusBarProps {
  stats: DashboardStats;
  meta: { target: string; duration: number; scanned: number } | null;
  scanning: boolean;
  progress: ScanProgress | null;
  native: boolean;
  version: string;
}

function Sep() {
  return <span className="h-3.5 w-px bg-line" aria-hidden />;
}

// Slim bottom status bar with live counters — the commercial-scanner
// replacement for a card dashboard.
export function StatusBar({ stats, meta, scanning, progress, native, version }: StatusBarProps) {
  const pct =
    progress && progress.total > 0 ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : null;

  return (
    <footer className="flex items-center gap-3 border-t border-line bg-surface px-3 py-1.5 text-xs text-muted">
      <span>
        <span className="font-semibold tabular-nums text-fg">{stats.total}</span> devices
      </span>
      <Sep />
      <span title="No vendor identified">
        <span className="tabular-nums text-fg">{stats.unknown}</span> unknown
      </span>
      <Sep />
      <span title="Port 3389 open" className={stats.openRdp > 0 ? "text-amber-600 dark:text-amber-400" : ""}>
        RDP <span className="tabular-nums">{stats.openRdp}</span>
      </span>
      <Sep />
      <span title="Port 445 open" className={stats.openSmb > 0 ? "text-amber-600 dark:text-amber-400" : ""}>
        SMB <span className="tabular-nums">{stats.openSmb}</span>
      </span>
      {stats.newDevices > 0 && (
        <>
          <Sep />
          <span className="text-brand-700 dark:text-brand-300" title="Not seen in the previous scan">
            <span className="tabular-nums">{stats.newDevices}</span> new
          </span>
        </>
      )}

      <span className="min-w-0 flex-1 truncate text-center text-faint">
        {scanning && progress
          ? progress.phase === "resolving"
            ? "Resolving names, MACs & vendors…"
            : pct != null
              ? `Scanning ${pct}% (${progress.done}/${progress.total})`
              : "Scanning…"
          : meta
            ? `${meta.target} — ${meta.scanned} scanned in ${(meta.duration / 1000).toFixed(1)}s`
            : "Ready"}
      </span>

      <span
        className={`inline-flex items-center gap-1.5 ${native ? "text-muted" : "text-amber-600 dark:text-amber-400"}`}
        title={native ? "Native backend active" : "Running in browser demo mode with mock data"}
      >
        {native ? <Wifi className="h-3.5 w-3.5" /> : <WifiOff className="h-3.5 w-3.5" />}
        {native ? "Live" : "Demo"}
      </span>
      <Sep />
      <span className="text-faint">v{version}</span>
    </footer>
  );
}
