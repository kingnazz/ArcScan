import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  LayoutGrid,
  ListTree,
  Moon,
  Radar,
  Sun,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { Logo } from "./components/Logo";
import { Dashboard } from "./components/Dashboard";
import { ScanControls } from "./components/ScanControls";
import { HostsTable } from "./components/HostsTable";
import { ScanHistory } from "./components/ScanHistory";
import { useTheme } from "./hooks/useTheme";
import { api } from "./lib/api";
import type {
  DashboardStats,
  ExportFormat,
  HostResult,
  ScanOptions,
  ScanProgress,
  ScanSummary,
} from "./types";

const APP_VERSION = "1.3.3";

type Tab = "results" | "history";

export default function App() {
  const { theme, toggle } = useTheme();
  const [hosts, setHosts] = useState<HostResult[]>([]);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("results");
  const [history, setHistory] = useState<ScanSummary[]>([]);
  const [activeScanId, setActiveScanId] = useState<number | null>(null);
  const [newIps, setNewIps] = useState<Set<string>>(new Set());
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [lastMeta, setLastMeta] = useState<{ target: string; duration: number; scanned: number } | null>(null);

  const refreshHistory = useCallback(async () => {
    try {
      setHistory(await api.listScans());
    } catch (e) {
      console.error("Failed to load history", e);
    }
  }, []);

  useEffect(() => {
    refreshHistory();
  }, [refreshHistory]);

  const runScan = useCallback(
    async (opts: ScanOptions) => {
      setScanning(true);
      setError(null);
      setActiveScanId(null);
      setHosts([]);
      setProgress({ done: 0, total: 0, phase: "probing" });
      try {
        // Snapshot the previous scan's IPs before saving the new one so we can
        // flag genuinely new devices.
        const previousIps = new Set(await api.lastScanIps());
        const result = await api.scan(opts, (p) => setProgress(p));

        const fresh = new Set(result.hosts.map((h) => h.ip).filter((ip) => !previousIps.has(ip)));
        setNewIps(previousIps.size === 0 ? new Set() : fresh);
        setHosts(result.hosts);
        setLastMeta({ target: result.target, duration: result.duration_ms, scanned: result.scanned });
        setTab("results");

        try {
          await api.save(result);
          await refreshHistory();
        } catch (e) {
          console.error("Failed to save scan", e);
        }
      } catch (e) {
        setError(String(e instanceof Error ? e.message : e));
      } finally {
        setScanning(false);
        setProgress(null);
      }
    },
    [refreshHistory],
  );

  const openScan = useCallback(async (id: number) => {
    try {
      const detail = await api.getScan(id);
      setHosts(detail.hosts);
      setActiveScanId(id);
      setNewIps(new Set());
      setLastMeta({ target: detail.target, duration: detail.duration_ms, scanned: detail.scanned });
      setTab("results");
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    }
  }, []);

  const deleteScan = useCallback(
    async (id: number) => {
      try {
        await api.deleteScan(id);
        if (activeScanId === id) setActiveScanId(null);
        await refreshHistory();
      } catch (e) {
        setError(String(e instanceof Error ? e.message : e));
      }
    },
    [activeScanId, refreshHistory],
  );

  const exportHosts = useCallback(
    async (format: ExportFormat) => {
      if (hosts.length === 0) return;
      const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
      try {
        await api.exportHosts(hosts, format, `arcscan-${stamp}.${format}`);
      } catch (e) {
        setError(String(e instanceof Error ? e.message : e));
      }
    },
    [hosts],
  );

  const stats: DashboardStats = useMemo(() => {
    return {
      total: hosts.length,
      unknown: hosts.filter((h) => !h.vendor).length,
      openRdp: hosts.filter((h) => h.open_ports.includes(3389)).length,
      openSmb: hosts.filter((h) => h.open_ports.includes(445)).length,
      newDevices: newIps.size,
    };
  }, [hosts, newIps]);

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <header className="flex items-center gap-3 border-b border-line bg-surface/70 px-5 py-3 backdrop-blur">
        <Logo size={30} />
        <div className="flex items-baseline gap-2">
          <h1 className="text-lg font-semibold tracking-tight text-fg">ArcScan</h1>
          <span className="text-[11px] font-medium text-faint">v{APP_VERSION}</span>
        </div>
        <span className="hidden text-xs text-muted sm:block">Fast, lightweight network &amp; port scanner</span>
        <div className="ml-auto flex items-center gap-2">
          <span
            className={`chip ${
              api.native
                ? "border-brand-500/30 bg-brand-500/10 text-brand-700 dark:text-brand-300"
                : "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300"
            }`}
            title={api.native ? "Native backend active" : "Running in browser demo mode with mock data"}
          >
            {api.native ? <Wifi className="h-3 w-3" /> : <WifiOff className="h-3 w-3" />}
            {api.native ? "Live" : "Demo"}
          </span>
          <button
            className="btn-icon"
            onClick={toggle}
            title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
            aria-label="Toggle theme"
          >
            {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </button>
        </div>
      </header>

      {/* Body */}
      <main className="flex min-h-0 flex-1 flex-col gap-3 p-4">
        <ScanControls scanning={scanning} onScan={runScan} />

        {scanning && progress && <ScanProgressBar progress={progress} />}

        {error && (
          <div className="flex items-start gap-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3.5 py-2.5 text-sm text-red-700 dark:text-red-200 animate-fade-in">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-red-500 dark:text-red-400" />
            <p className="flex-1 leading-relaxed">{error}</p>
            <button className="btn-icon hover:text-red-500 dark:hover:text-red-200" onClick={() => setError(null)} title="Dismiss">
              <X className="h-4 w-4" />
            </button>
          </div>
        )}

        <Dashboard stats={stats} />

        {/* Tabs */}
        <div className="flex items-center justify-between">
          <div className="flex gap-1 rounded-lg border border-line bg-surface p-1">
            <TabButton active={tab === "results"} onClick={() => setTab("results")} icon={<LayoutGrid className="h-4 w-4" />}>
              Results
            </TabButton>
            <TabButton active={tab === "history"} onClick={() => setTab("history")} icon={<ListTree className="h-4 w-4" />}>
              History
            </TabButton>
          </div>
          {lastMeta && tab === "results" && (
            <div className="hidden items-center gap-2 text-xs text-muted sm:flex">
              <Radar className="h-3.5 w-3.5 text-brand-500" />
              <span className="font-mono text-fg">{lastMeta.target}</span>
              <span>·</span>
              <span>
                {hosts.length} live / {lastMeta.scanned} scanned
              </span>
              <span>·</span>
              <span>{(lastMeta.duration / 1000).toFixed(1)}s</span>
              {activeScanId != null && <span className="text-brand-600 dark:text-brand-300">· from history</span>}
            </div>
          )}
        </div>

        {tab === "results" ? (
          <HostsTable hosts={hosts} newIps={newIps} onExport={exportHosts} />
        ) : (
          <ScanHistory scans={history} activeId={activeScanId} onOpen={openScan} onDelete={deleteScan} />
        )}
      </main>
    </div>
  );
}

function ScanProgressBar({ progress }: { progress: ScanProgress }) {
  const pct = progress.total > 0 ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : 0;
  const label =
    progress.phase === "resolving"
      ? "Resolving names, MACs & ARP…"
      : progress.phase === "done"
        ? "Finishing…"
        : "Probing hosts…";
  const indeterminate = progress.total === 0 || progress.phase === "resolving";
  return (
    <div className="panel px-4 py-3 animate-fade-in">
      <div className="mb-2 flex items-center justify-between text-xs">
        <span className="font-medium text-fg">{label}</span>
        {progress.total > 0 && progress.phase === "probing" && (
          <span className="tabular-nums text-muted">
            {pct}% <span className="ml-1 text-faint">({progress.done}/{progress.total})</span>
          </span>
        )}
      </div>
      <div className="h-2 w-full overflow-hidden rounded-full bg-surface2">
        <div
          className={`h-full rounded-full bg-brand-500 transition-[width] duration-200 ease-out ${
            indeterminate ? "w-1/3 animate-pulse" : ""
          }`}
          style={indeterminate ? undefined : { width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  icon,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={`inline-flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors ${
        active ? "bg-brand-500/15 text-brand-700 dark:text-brand-200" : "text-muted hover:text-fg"
      }`}
    >
      {icon}
      {children}
    </button>
  );
}
