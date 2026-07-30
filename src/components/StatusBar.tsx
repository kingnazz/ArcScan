// The bottom status bar.
//
// Doubles as the scan progress display, which keeps the progress indicator out of
// the results area. While a scan runs it reads like a sentence: "14 devices found
// · 172 of 254 checked · Resolving names and vendors · 8.1 s".

import { CircleSlash, Wifi } from "lucide-react";
import { formatCount, formatDuration, phaseLabel } from "../lib/format";
import type { ScanMeta, ScanMode } from "../hooks/useLiveScan";
import type { ScanProgress } from "../types";

export interface StatusBarProps {
  mode: ScanMode;
  progress: ScanProgress | null;
  meta: ScanMeta | null;
  deviceCount: number;
  version: string;
  native: boolean;
}

export function StatusBar({ mode, progress, meta, deviceCount, version, native }: StatusBarProps) {
  return (
    <footer className="flex h-[26px] shrink-0 items-center gap-2.5 border-t border-border bg-surface-raised px-3 text-xs text-text-secondary">
      <span
        // Polite, so a screen reader hears the counts without every progress tick
        // interrupting.
        aria-live="polite"
        aria-atomic="true"
        className="min-w-0 flex-1 truncate"
      >
        {mode === "scanning" && progress ? (
          <>
            <span className="font-medium text-text">{formatCount(progress.found)}</span>{" "}
            {progress.found === 1 ? "device" : "devices"} found
            {progress.total > 0 ? (
              <>
                {" · "}
                <span className="text-text">{formatCount(progress.done)}</span> of{" "}
                {formatCount(progress.total)} checked
                <span className="text-text-muted">
                  {" "}
                  ({Math.min(100, Math.round((progress.done / progress.total) * 100))}%)
                </span>
              </>
            ) : null}
            {" · "}
            {phaseLabel(progress.phase)}
            {" · "}
            <span className="text-text-muted">{formatDuration(progress.elapsed_ms)}</span>
          </>
        ) : meta ? (
          <>
            <span className="font-medium text-text">{formatCount(deviceCount)}</span>{" "}
            {deviceCount === 1 ? "device" : "devices"}
            {" · "}
            <span className="mono">{meta.target}</span>
            {" · "}
            {meta.cancelled
              ? `stopped after ${formatCount(meta.probed)} of ${formatCount(meta.scanned)} addresses`
              : `${formatCount(meta.scanned)} addresses in ${formatDuration(meta.durationMs)}`}
            {mode === "history" ? " · viewing a saved scan" : ""}
          </>
        ) : (
          "Ready"
        )}
      </span>

      {native ? (
        <span
          className="inline-flex shrink-0 items-center gap-1 text-text-muted"
          title="Scanning runs locally on this computer"
        >
          <Wifi className="h-3 w-3" aria-hidden />
          Local
        </span>
      ) : (
        <span
          className="inline-flex shrink-0 items-center gap-1 text-warning"
          title="Running in a browser with a built-in demo network. Install the desktop app to scan a real network."
        >
          <CircleSlash className="h-3 w-3" aria-hidden />
          Demo data
        </span>
      )}

      <span className="shrink-0 text-text-muted">v{version}</span>
    </footer>
  );
}

/**
 * The thin progress strip under the command bar.
 *
 * Determinate while addresses are being probed, and a small travelling bar for the
 * phases with no countable unit of work. Nothing else on screen animates during a
 * scan, so a streaming table stays readable.
 */
export function ProgressStrip({
  scanning,
  progress,
}: {
  scanning: boolean;
  progress: ScanProgress | null;
}) {
  const determinate =
    scanning && progress != null && progress.total > 0 && progress.phase === "probing";
  const percent = determinate
    ? Math.min(100, Math.round((progress.done / progress.total) * 100))
    : 0;

  return (
    <div
      className="h-[2px] w-full shrink-0 overflow-hidden bg-surface-sunken"
      role={scanning ? "progressbar" : undefined}
      aria-valuenow={determinate ? percent : undefined}
      aria-valuemin={determinate ? 0 : undefined}
      aria-valuemax={determinate ? 100 : undefined}
      aria-label={scanning ? "Scan progress" : undefined}
    >
      {scanning ? (
        determinate ? (
          <div
            className="h-full bg-accent transition-[width] duration-slow ease-out"
            style={{ width: `${percent}%` }}
          />
        ) : (
          <div className="animate-indeterminate h-full w-1/4 bg-accent" />
        )
      ) : null}
    </div>
  );
}
