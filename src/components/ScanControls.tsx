import { useState } from "react";
import { Loader2, Radar, Settings2, ShieldCheck } from "lucide-react";
import { Toggle } from "./Toggle";
import { DEFAULT_PORTS, type ScanOptions } from "../types";
import { parsePorts } from "../lib/format";

interface ScanControlsProps {
  scanning: boolean;
  onScan: (opts: ScanOptions) => void;
  onStop?: () => void;
}

export function ScanControls({ scanning, onScan }: ScanControlsProps) {
  const [target, setTarget] = useState("192.168.1.0/24");
  const [authorized, setAuthorized] = useState(false);
  const [allowPublic, setAllowPublic] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [timeoutMs, setTimeoutMs] = useState(600);
  const [concurrency, setConcurrency] = useState(128);
  const [portsInput, setPortsInput] = useState(DEFAULT_PORTS.join(", "));

  const canScan = authorized && target.trim().length > 0 && !scanning;

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!canScan) return;
    const ports = parsePorts(portsInput);
    onScan({
      target: target.trim(),
      ports: ports.length ? ports : DEFAULT_PORTS,
      timeout_ms: timeoutMs,
      concurrency,
      allow_public: allowPublic,
      authorized,
    });
  }

  return (
    <form onSubmit={submit} className="panel p-4">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end">
        <div className="flex-1">
          <label htmlFor="target" className="mb-1.5 block text-xs font-medium text-slate-400">
            Target range
          </label>
          <input
            id="target"
            className="input font-mono"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            placeholder="192.168.1.0/24  ·  10.0.0.1-50  ·  192.168.1.20"
            spellCheck={false}
            autoComplete="off"
          />
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setShowAdvanced((v) => !v)}
            className={`btn-ghost ${showAdvanced ? "border-brand-400/40 text-brand-200" : ""}`}
            aria-expanded={showAdvanced}
          >
            <Settings2 className="h-4 w-4" />
            Advanced
          </button>
          <button type="submit" className="btn-primary min-w-[120px]" disabled={!canScan}>
            {scanning ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                Scanning…
              </>
            ) : (
              <>
                <Radar className="h-4 w-4" />
                Scan
              </>
            )}
          </button>
        </div>
      </div>

      <div className="mt-3 flex flex-col gap-3 border-t border-white/5 pt-3 sm:flex-row sm:items-center sm:justify-between">
        <label
          className={`flex cursor-pointer items-center gap-2.5 rounded-lg border px-3 py-2 text-sm transition-colors ${
            authorized
              ? "border-brand-400/40 bg-brand-500/10 text-brand-100"
              : "border-white/10 bg-white/5 text-slate-300 hover:border-white/20"
          }`}
        >
          <input
            type="checkbox"
            className="peer sr-only"
            checked={authorized}
            onChange={(e) => setAuthorized(e.target.checked)}
          />
          <span
            className={`flex h-4 w-4 items-center justify-center rounded border ${
              authorized ? "border-brand-400 bg-brand-500" : "border-slate-500"
            }`}
          >
            {authorized && <ShieldCheck className="h-3 w-3 text-white" />}
          </span>
          I am authorized to scan this network
        </label>

        <Toggle
          checked={allowPublic}
          onChange={setAllowPublic}
          tone="warning"
          label="Allow public range"
          description="Off = private RFC1918 only (recommended)"
        />
      </div>

      {showAdvanced && (
        <div className="mt-3 grid grid-cols-1 gap-3 rounded-lg border border-white/5 bg-arc-900/40 p-3 sm:grid-cols-3 animate-fade-in">
          <div>
            <label htmlFor="timeout" className="mb-1 block text-xs font-medium text-slate-400">
              Per-host timeout: <span className="text-brand-200">{timeoutMs} ms</span>
            </label>
            <input
              id="timeout"
              type="range"
              min={100}
              max={3000}
              step={50}
              value={timeoutMs}
              onChange={(e) => setTimeoutMs(Number(e.target.value))}
              className="w-full accent-brand-400"
            />
          </div>
          <div>
            <label htmlFor="concurrency" className="mb-1 block text-xs font-medium text-slate-400">
              Max concurrency: <span className="text-brand-200">{concurrency}</span>
            </label>
            <input
              id="concurrency"
              type="range"
              min={8}
              max={1024}
              step={8}
              value={concurrency}
              onChange={(e) => setConcurrency(Number(e.target.value))}
              className="w-full accent-brand-400"
            />
          </div>
          <div>
            <label htmlFor="ports" className="mb-1 block text-xs font-medium text-slate-400">
              TCP ports
            </label>
            <input
              id="ports"
              className="input font-mono text-xs"
              value={portsInput}
              onChange={(e) => setPortsInput(e.target.value)}
              placeholder="22, 80, 443, 445, 3389, 8080"
              spellCheck={false}
            />
          </div>
        </div>
      )}
    </form>
  );
}
