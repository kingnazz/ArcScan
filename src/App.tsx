import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  Clock,
  DownloadCloud,
  LayoutGrid,
  Moon,
  Sun,
  X,
} from "lucide-react";
import { Logo } from "./components/Logo";
import { StatusBar } from "./components/StatusBar";
import { ScanControls } from "./components/ScanControls";
import { HostsTable } from "./components/HostsTable";
import { ScanHistory } from "./components/ScanHistory";
import { useTheme } from "./hooks/useTheme";
import { useUpdater } from "./hooks/useUpdater";
import { api } from "./lib/api";
import {
  type KnownMap,
  loadKnown,
  loadRanges,
  pushRange,
  setLabel as prefsSetLabel,
  toggleKnown as prefsToggleKnown,
} from "./lib/prefs";
import type {
  DashboardStats,
  ExportFormat,
  HostResult,
  ScanOptions,
  ScanProgress,
  ScanSummary,
} from "./types";

const APP_VERSION = "1.6.2";

type Tab = "results" | "history";

export default function App() {
  const { theme, toggle } = useTheme();
  const updater = useUpdater();
  const [hosts, setHosts] = useState<HostResult[]>([]);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("results");
  const [history, setHistory] = useState<ScanSummary[]>([]);
  const [activeScanId, setActiveScanId] = useState<number | null>(null);
  const [newIps, setNewIps] = useState<Set<string>>(new Set());
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [lastMeta, setLastMeta] = useState<{ target: string; duration: number; scanned: number } | null>(null);
  const [known, setKnown] = useState<KnownMap>({});
  const [recents, setRecents] = useState<string[]>([]);
  const [lastOpts, setLastOpts] = useState<ScanOptions | null>(null);

  useEffect(() => {
    setKnown(loadKnown());
    setRecents(loadRanges());
  }, []);

  const toggleKnown = useCallback((mac: string, defaultLabel = "") => {
    setKnown((m) => prefsToggleKnown(m, mac, defaultLabel));
  }, []);

  const setDeviceLabel = useCallback((mac: string, label: string) => {
    setKnown((m) => prefsSetLabel(m, mac, label));
  }, []);

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
      setLastOpts(opts);
      setRecents(pushRange(opts.target));
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

  const rescan = useCallback(() => {
    if (lastOpts && !scanning) runScan(lastOpts);
  }, [lastOpts, scanning, runScan]);

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
      {/* Title bar */}
      <header className="flex items-center gap-2.5 border-b border-line bg-surface px-3 py-1.5">
        <Logo size={22} />
        <h1 className="text-[13px] font-semibold tracking-tight text-fg">ArcScan</h1>
        <span className="hidden text-xs text-faint sm:block">Network &amp; port scanner</span>
        <div className="ml-auto flex items-center gap-1">
          <button
            className="btn-icon"
            onClick={() => (api.native ? updater.check(true) : api.checkForUpdates())}
            title="Check for updates"
            aria-label="Check for updates"
          >
            <DownloadCloud className={`h-4 w-4 ${updater.status === "checking" ? "animate-pulse" : ""}`} />
          </button>
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

      {/* Toolbar */}
      <ScanControls
        scanning={scanning}
        onScan={runScan}
        onRescan={rescan}
        canRescan={lastOpts != null}
        recents={recents}
      />

      {/* Thin progress strip under the toolbar while scanning */}
      <div className="h-[3px] w-full bg-surface2" aria-hidden={!scanning}>
        {scanning && progress && (
          <div
            className={`h-full bg-brand-500 transition-[width] duration-200 ease-out ${
              progress.total === 0 || progress.phase !== "probing" ? "w-1/3 animate-pulse" : ""
            }`}
            style={
              progress.total > 0 && progress.phase === "probing"
                ? { width: `${Math.min(100, Math.round((progress.done / progress.total) * 100))}%` }
                : undefined
            }
          />
        )}
      </div>

      <UpdateBanner updater={updater} />

      {error && (
        <div className="flex items-start gap-3 border-b border-red-500/30 bg-red-500/10 px-3.5 py-2 text-sm text-red-700 dark:text-red-200 animate-fade-in">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-red-500 dark:text-red-400" />
          <p className="flex-1 leading-relaxed">{error}</p>
          <button className="btn-icon hover:text-red-500 dark:hover:text-red-200" onClick={() => setError(null)} title="Dismiss">
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      {/* Tab strip */}
      <div className="flex items-center border-b border-line bg-surface px-2">
        <button className={`tab ${tab === "results" ? "tab-active" : ""}`} onClick={() => setTab("results")}>
          <LayoutGrid className="h-3.5 w-3.5" />
          Scan list
        </button>
        <button className={`tab ${tab === "history" ? "tab-active" : ""}`} onClick={() => setTab("history")}>
          <Clock className="h-3.5 w-3.5" />
          History
        </button>
        {activeScanId != null && tab === "results" && (
          <span className="ml-auto pr-2 text-xs text-brand-700 dark:text-brand-300">viewing saved scan</span>
        )}
      </div>

      {/* Content */}
      {tab === "results" ? (
        <HostsTable
          hosts={hosts}
          newIps={newIps}
          onExport={exportHosts}
          known={known}
          onToggleKnown={toggleKnown}
          onSetLabel={setDeviceLabel}
        />
      ) : (
        <ScanHistory scans={history} activeId={activeScanId} onOpen={openScan} onDelete={deleteScan} />
      )}

      <StatusBar
        stats={stats}
        meta={lastMeta}
        scanning={scanning}
        progress={progress}
        native={api.native}
        version={APP_VERSION}
      />
    </div>
  );
}

function UpdateBanner({ updater }: { updater: ReturnType<typeof useUpdater> }) {
  const { status, version, progress, error, install, dismiss } = updater;
  if (status === "idle" || status === "checking") return null;

  const busy = status === "downloading" || status === "installing";

  return (
    <div className="flex items-center gap-3 border-b border-brand-400/30 bg-brand-500/10 px-3.5 py-2 text-sm animate-fade-in">
      <DownloadCloud className="h-4 w-4 shrink-0 text-brand-600 dark:text-brand-300" />
      <div className="flex-1 leading-relaxed">
        {status === "available" && (
          <span className="text-fg">
            <span className="font-semibold">ArcScan v{version}</span> is available.
          </span>
        )}
        {status === "downloading" && (
          <span className="text-fg">Downloading update… {progress > 0 ? `${progress}%` : ""}</span>
        )}
        {status === "installing" && <span className="text-fg">Installing… ArcScan will restart.</span>}
        {status === "uptodate" && <span className="text-muted">You're on the latest version.</span>}
        {status === "error" && (
          <span className="text-amber-700 dark:text-amber-300">Update check failed: {error}</span>
        )}
      </div>
      {status === "downloading" && (
        <div className="hidden h-1.5 w-40 overflow-hidden rounded-full bg-surface2 sm:block">
          <div className="h-full rounded-full bg-brand-500 transition-[width]" style={{ width: `${progress}%` }} />
        </div>
      )}
      {status === "available" && (
        <button className="btn-primary" onClick={install}>
          <DownloadCloud className="h-4 w-4" />
          Update now
        </button>
      )}
      {!busy && (
        <button className="btn-icon" onClick={dismiss} title="Dismiss">
          <X className="h-4 w-4" />
        </button>
      )}
    </div>
  );
}
