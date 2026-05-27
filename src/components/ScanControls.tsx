import { useMemo, useState } from "react";
import { AlertTriangle, Loader2, Play, Radar, Settings2, Square } from "lucide-react";
import { parseTarget } from "../lib/ip";
import { DEFAULT_PORTS } from "../types";
import type { ScanOptions } from "../types";
import type { ScanState } from "../hooks/useScan";

interface ScanControlsProps {
  state: ScanState;
  authorized: boolean;
  onScan: (options: ScanOptions) => void;
  onCancel: () => void;
}

export function ScanControls({ state, authorized, onScan, onCancel }: ScanControlsProps) {
  const [target, setTarget] = useState("192.168.1.0/24");
  const [allowPublic, setAllowPublic] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [timeoutMs, setTimeoutMs] = useState(600);
  const [concurrency, setConcurrency] = useState(128);

  const parsed = useMemo(() => parseTarget(target), [target]);
  const scanning = state === "scanning";

  const publicBlocked = parsed.ok && !parsed.allPrivate && !allowPublic;
  const canScan =
    parsed.ok && authorized && !scanning && !publicBlocked;

  const submit = () => {
    if (!canScan) return;
    onScan({
      target: target.trim(),
      timeoutMs,
      concurrency,
      ports: DEFAULT_PORTS,
      allowPublic,
      authorized,
    });
  };

  return (
    <div className="panel p-4">
      <div className="flex items-center gap-3">
        <div className="relative flex-1">
          <Radar className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-slate-500" />
          <input
            className="input w-full pl-9 font-mono"
            placeholder="192.168.1.0/24  ·  10.0.0.1-10.0.0.50"
            value={target}
            spellCheck={false}
            onChange={(e) => setTarget(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </div>

        {scanning ? (
          <button className="btn-outline" onClick={onCancel}>
            <Square className="h-4 w-4" />
            Stop
          </button>
        ) : (
          <button className="btn-primary" disabled={!canScan} onClick={submit}>
            <Play className="h-4 w-4" />
            Scan
          </button>
        )}

        <button
          className={`icon-btn ${showAdvanced ? "bg-base-700 text-white" : ""}`}
          title="Scan options"
          onClick={() => setShowAdvanced((s) => !s)}
        >
          {scanning ? <Loader2 className="h-4 w-4 animate-spin" /> : <Settings2 className="h-4 w-4" />}
        </button>
      </div>

      {/* Validation + range summary line */}
      <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs min-h-[1rem]">
        {parsed.error ? (
          <span className="text-danger">{parsed.error}</span>
        ) : parsed.ok ? (
          <span className="text-slate-400">
            <span className="text-slate-200 font-medium tabular-nums">
              {parsed.count.toLocaleString()}
            </span>{" "}
            addresses · {parsed.first} → {parsed.last} ·{" "}
            {parsed.allPrivate ? (
              <span className="text-ok">private RFC1918</span>
            ) : (
              <span className="text-warn">public range</span>
            )}
          </span>
        ) : null}
        {!authorized && (
          <span className="text-warn">Acknowledge authorization to enable scanning.</span>
        )}
      </div>

      {/* Public-range guard */}
      {parsed.ok && !parsed.allPrivate && (
        <label className="mt-2 flex items-start gap-2 rounded-lg bg-warn/10 border border-warn/30 px-3 py-2 cursor-pointer">
          <AlertTriangle className="h-4 w-4 text-warn mt-0.5 shrink-0" />
          <span className="text-xs text-slate-300 flex-1">
            This target falls outside private RFC1918 space. Scanning public addresses
            requires extra care and explicit authorization.
            <input
              type="checkbox"
              checked={allowPublic}
              onChange={(e) => setAllowPublic(e.target.checked)}
              className="ml-2 align-middle h-3.5 w-3.5 rounded border-base-600 bg-base-850 text-warn focus:ring-warn/40"
            />
            <span className="ml-1.5 align-middle text-slate-200">Allow public range</span>
          </span>
        </label>
      )}

      {showAdvanced && (
        <div className="mt-3 grid grid-cols-2 gap-4 border-t border-base-700/70 pt-3 animate-fade-in">
          <label className="block">
            <span className="text-xs text-slate-400">Per-host timeout</span>
            <div className="flex items-center gap-2 mt-1">
              <input
                type="range"
                min={150}
                max={2000}
                step={50}
                value={timeoutMs}
                onChange={(e) => setTimeoutMs(Number(e.target.value))}
                className="flex-1 accent-accent"
              />
              <span className="text-xs tabular-nums text-slate-300 w-16 text-right">
                {timeoutMs} ms
              </span>
            </div>
          </label>
          <label className="block">
            <span className="text-xs text-slate-400">Max concurrency</span>
            <div className="flex items-center gap-2 mt-1">
              <input
                type="range"
                min={16}
                max={512}
                step={16}
                value={concurrency}
                onChange={(e) => setConcurrency(Number(e.target.value))}
                className="flex-1 accent-accent"
              />
              <span className="text-xs tabular-nums text-slate-300 w-16 text-right">
                {concurrency}
              </span>
            </div>
          </label>
          <div className="col-span-2 text-xs text-slate-500">
            TCP service probe ports: {DEFAULT_PORTS.join(", ")}
          </div>
        </div>
      )}
    </div>
  );
}
