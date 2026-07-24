import { useEffect, useRef, useState } from "react";
import { Loader2, Locate, Play, RotateCw, Settings2 } from "lucide-react";
import { DEFAULT_PORTS, type ScanOptions } from "../types";
import { api } from "../lib/api";
import { parsePorts } from "../lib/format";

interface ScanControlsProps {
  scanning: boolean;
  onScan: (opts: ScanOptions) => void;
  onRescan?: () => void;
  canRescan?: boolean;
  recents?: string[];
}

// Single-row toolbar: Scan button, target field, detect, and an Options
// popover for timeout/concurrency/ports — the classic scanner layout.
export function ScanControls({ scanning, onScan, onRescan, canRescan = false, recents = [] }: ScanControlsProps) {
  const [target, setTarget] = useState("192.168.1.0/24");
  const [timeoutMs, setTimeoutMs] = useState(900);
  const [concurrency, setConcurrency] = useState(64);
  const [portsInput, setPortsInput] = useState(DEFAULT_PORTS.join(", "));
  const [localIp, setLocalIp] = useState<string | null>(null);
  const [detecting, setDetecting] = useState(false);
  const userEditedTarget = useRef(false);

  // Fill in the local subnet as a helpful default when it can be detected.
  async function detect(applyTarget: boolean) {
    setDetecting(true);
    try {
      const nets = await api.detectNetworks();
      if (nets.length > 0) {
        setLocalIp(nets[0].ip);
        if (applyTarget) {
          setTarget(nets[0].cidr);
          userEditedTarget.current = false;
        }
      }
    } catch (e) {
      console.error("network detection failed", e);
    } finally {
      setDetecting(false);
    }
  }

  // Auto-detect on first mount, but never clobber a target the user has typed.
  useEffect(() => {
    detect(!userEditedTarget.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
    <form onSubmit={submit} className="flex items-center gap-2 border-b border-line bg-surface px-3 py-2">
      <button type="submit" className="btn-primary min-w-[104px]" disabled={!canScan}>
        {scanning ? (
          <>
            <Loader2 className="h-4 w-4 animate-spin" />
            Scanning
          </>
        ) : (
          <>
            <Play className="h-4 w-4" />
            Scan
          </>
        )}
      </button>

      <div className="flex min-w-0 max-w-xl flex-1 items-center gap-2">
        <input
          id="target"
          className="input font-mono"
          value={target}
          onChange={(e) => {
            userEditedTarget.current = true;
            setTarget(e.target.value);
          }}
          placeholder="192.168.1.0/24  ·  10.0.0.1-50  ·  192.168.1.20"
          spellCheck={false}
          autoComplete="off"
          list="arcscan-recent-ranges"
          aria-label="Target — IP, range, or CIDR"
          title="Target — single IP, dashed range, or CIDR"
        />
        {recents.length > 0 && (
          <datalist id="arcscan-recent-ranges">
            {recents.map((r) => (
              <option key={r} value={r} />
            ))}
          </datalist>
        )}
      </div>

      <button
        type="button"
        onClick={() => detect(true)}
        className="btn-ghost shrink-0"
        title="Auto-detect this device's local network"
        disabled={detecting}
      >
        {detecting ? <Loader2 className="h-4 w-4 animate-spin" /> : <Locate className="h-4 w-4" />}
        Detect
      </button>

      {onRescan && (
        <button
          type="button"
          className="btn-ghost shrink-0"
          onClick={onRescan}
          disabled={!canRescan || scanning}
          title="Rescan the last target"
        >
          <RotateCw className={`h-4 w-4 ${scanning ? "animate-spin" : ""}`} />
          Rescan
        </button>
      )}

      <div className="mx-1 h-6 w-px shrink-0 bg-line" aria-hidden />

      <OptionsMenu
        timeoutMs={timeoutMs}
        setTimeoutMs={setTimeoutMs}
        concurrency={concurrency}
        setConcurrency={setConcurrency}
        portsInput={portsInput}
        setPortsInput={setPortsInput}
      />

      {localIp && (
        <span className="ml-auto hidden shrink-0 text-xs text-muted lg:block">
          This device: <span className="font-mono font-medium text-fg">{localIp}</span>
        </span>
      )}
    </form>
  );
}

function OptionsMenu({
  timeoutMs,
  setTimeoutMs,
  concurrency,
  setConcurrency,
  portsInput,
  setPortsInput,
}: {
  timeoutMs: number;
  setTimeoutMs: (v: number) => void;
  concurrency: number;
  setConcurrency: (v: number) => void;
  portsInput: string;
  setPortsInput: (v: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDown(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div className="relative shrink-0" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={`btn-ghost ${open ? "bg-surface2" : ""}`}
        aria-expanded={open}
        title="Scan options — ports, timeout, concurrency"
      >
        <Settings2 className="h-4 w-4" />
        Options
      </button>
      {open && (
        <div className="absolute right-0 z-30 mt-1 w-80 rounded-md border border-line bg-surface p-3 shadow-panel animate-fade-in">
          <div className="mb-3">
            <label htmlFor="ports" className="mb-1 block text-xs font-medium text-muted">
              TCP ports <span className="text-faint">— lists &amp; ranges, e.g. 1-1024</span>
            </label>
            <input
              id="ports"
              className="input font-mono text-xs"
              value={portsInput}
              onChange={(e) => setPortsInput(e.target.value)}
              placeholder="22, 80, 443, 3389, 8000-8100"
              spellCheck={false}
            />
          </div>
          <div className="mb-3">
            <label htmlFor="timeout" className="mb-1 block text-xs font-medium text-muted">
              Per-host timeout: <span className="font-semibold text-fg">{timeoutMs} ms</span>
            </label>
            <input
              id="timeout"
              type="range"
              min={100}
              max={3000}
              step={50}
              value={timeoutMs}
              onChange={(e) => setTimeoutMs(Number(e.target.value))}
              className="w-full accent-brand-600"
            />
          </div>
          <div>
            <label htmlFor="concurrency" className="mb-1 block text-xs font-medium text-muted">
              Max concurrency: <span className="font-semibold text-fg">{concurrency}</span>
            </label>
            <input
              id="concurrency"
              type="range"
              min={8}
              max={1024}
              step={8}
              value={concurrency}
              onChange={(e) => setConcurrency(Number(e.target.value))}
              className="w-full accent-brand-600"
            />
          </div>
        </div>
      )}
    </div>
  );
}
