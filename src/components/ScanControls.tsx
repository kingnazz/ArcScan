import { useState } from "react";
import { Loader2, Radar, Settings2 } from "lucide-react";
import { DEFAULT_PORTS, type ScanOptions } from "../types";
import { parsePorts } from "../lib/format";

interface ScanControlsProps {
  scanning: boolean;
  onScan: (opts: ScanOptions) => void;
}

export function ScanControls({ scanning, onScan }: ScanControlsProps) {
  const [target, setTarget] = useState("192.168.1.0/24");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [timeoutMs, setTimeoutMs] = useState(600);
  const [concurrency, setConcurrency] = useState(128);
  const [portsInput, setPortsInput] = useState(DEFAULT_PORTS.join(", "));

  const canScan = target.trim().length > 0 && !scanning;

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!canScan) return;
    const ports = parsePorts(portsInput);
    onScan({
      target: target.trim(),
      ports: ports.length ? ports : DEFAULT_PORTS,
      timeout_ms: timeoutMs,
      concurrency,
    });
  }

  return (
    <form onSubmit={submit} className="panel p-4">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end">
        <div className="flex-1">
          <label htmlFor="target" className="mb-1.5 block text-xs font-medium text-muted">
            Target — IP, range, or CIDR
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
            className={`btn-ghost ${showAdvanced ? "border-brand-400/50 text-brand-600" : ""}`}
            aria-expanded={showAdvanced}
          >
            <Settings2 className="h-4 w-4" />
            Advanced
          </button>
          <button type="submit" className="btn-primary min-w-[124px]" disabled={!canScan}>
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

      {showAdvanced && (
        <div className="mt-3 grid grid-cols-1 gap-3 rounded-lg border border-line bg-surface2 p-3 sm:grid-cols-3 animate-fade-in">
          <div>
            <label htmlFor="timeout" className="mb-1 block text-xs font-medium text-muted">
              Per-host timeout: <span className="text-brand-600">{timeoutMs} ms</span>
            </label>
            <input
              id="timeout"
              type="range"
              min={100}
              max={3000}
              step={50}
              value={timeoutMs}
              onChange={(e) => setTimeoutMs(Number(e.target.value))}
              className="w-full accent-brand-500"
            />
          </div>
          <div>
            <label htmlFor="concurrency" className="mb-1 block text-xs font-medium text-muted">
              Max concurrency: <span className="text-brand-600">{concurrency}</span>
            </label>
            <input
              id="concurrency"
              type="range"
              min={8}
              max={1024}
              step={8}
              value={concurrency}
              onChange={(e) => setConcurrency(Number(e.target.value))}
              className="w-full accent-brand-500"
            />
          </div>
          <div>
            <label htmlFor="ports" className="mb-1 block text-xs font-medium text-muted">
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
