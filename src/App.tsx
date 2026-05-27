import { useCallback, useEffect, useState } from "react";
import { Download, Radar, ScanLine } from "lucide-react";
import { AuthorizationBanner } from "./components/AuthorizationBanner";
import { Dashboard } from "./components/Dashboard";
import { ProgressBar } from "./components/ProgressBar";
import { ResultsTable } from "./components/ResultsTable";
import { ScanControls } from "./components/ScanControls";
import { ScanHistory } from "./components/ScanHistory";
import { useScan } from "./hooks/useScan";
import { deleteScan, exportCsv, getScanHosts, isTauri, listScans } from "./lib/api";
import { hostsToCsv } from "./lib/csv";
import type { ScanOptions, ScanSummary } from "./types";

export default function App() {
  const scan = useScan();
  const [authorized, setAuthorized] = useState(false);
  const [history, setHistory] = useState<ScanSummary[]>([]);
  const [activeScanId, setActiveScanId] = useState<number | null>(null);
  const [exporting, setExporting] = useState(false);

  const refreshHistory = useCallback(async () => {
    setHistory(await listScans());
  }, []);

  useEffect(() => {
    void refreshHistory();
  }, [refreshHistory]);

  const handleScan = useCallback(
    async (options: ScanOptions) => {
      setActiveScanId(null);
      await scan.start(options);
      await refreshHistory();
    },
    [scan, refreshHistory]
  );

  // After a scan finishes, mark the freshly persisted scan as active.
  useEffect(() => {
    if (scan.state === "done" && scan.lastResult?.scanId != null) {
      setActiveScanId(scan.lastResult.scanId);
    }
  }, [scan.state, scan.lastResult]);

  const handleSelectHistory = useCallback(
    async (summary: ScanSummary) => {
      setActiveScanId(summary.id);
      const hosts = await getScanHosts(summary.id);
      scan.loadHosts(hosts, {
        scanId: summary.id,
        target: summary.target,
        startedAt: summary.startedAt,
        finishedAt: summary.finishedAt,
        hosts,
        totalScanned: summary.totalScanned,
      });
    },
    [scan]
  );

  const handleDelete = useCallback(
    async (id: number) => {
      await deleteScan(id);
      if (activeScanId === id) {
        setActiveScanId(null);
        scan.loadHosts([]);
      }
      await refreshHistory();
    },
    [activeScanId, scan, refreshHistory]
  );

  const handleExport = useCallback(async () => {
    if (scan.hosts.length === 0) return;
    setExporting(true);
    try {
      const csv = hostsToCsv(scan.hosts);
      const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
      const name = `arcscan-${scan.lastResult?.target?.replace(/[^\w.-]/g, "_") ?? "scan"}-${stamp}.csv`;
      await exportCsv(csv, name);
    } finally {
      setExporting(false);
    }
  }, [scan.hosts, scan.lastResult]);

  return (
    <div className="h-full flex bg-base-950 text-slate-200">
      {/* Sidebar */}
      <aside className="w-64 shrink-0 flex flex-col border-r border-base-800 bg-base-900/60">
        <div className="flex items-center gap-2.5 px-4 h-14 border-b border-base-800">
          <div className="grid place-items-center h-8 w-8 rounded-lg bg-accent/15 text-accent-soft">
            <Radar className="h-5 w-5" />
          </div>
          <div>
            <div className="font-semibold text-white leading-none tracking-tight">ArcScan</div>
            <div className="text-[10px] text-slate-500 mt-0.5">LAN Discovery for MSPs</div>
          </div>
        </div>
        <ScanHistory
          scans={history}
          activeId={activeScanId}
          onSelect={handleSelectHistory}
          onDelete={handleDelete}
        />
        <div className="px-4 py-2 border-t border-base-800 text-[10px] text-slate-600">
          {isTauri() ? "Desktop runtime" : "Browser preview (mock data)"}
        </div>
      </aside>

      {/* Main */}
      <main className="flex-1 flex flex-col min-w-0">
        <header className="h-14 shrink-0 flex items-center gap-4 px-5 border-b border-base-800">
          <div className="flex items-center gap-2 text-slate-300">
            <ScanLine className="h-4 w-4 text-accent-soft" />
            <h1 className="text-sm font-medium">Network Inventory</h1>
          </div>
          <div className="flex-1 max-w-xl">
            <ProgressBar progress={scan.progress} active={scan.state === "scanning"} />
          </div>
          <button
            className="btn-outline"
            disabled={scan.hosts.length === 0 || exporting}
            onClick={handleExport}
          >
            <Download className="h-4 w-4" />
            Export CSV
          </button>
        </header>

        <div className="flex-1 flex flex-col min-h-0 gap-3 p-5 overflow-hidden">
          <AuthorizationBanner authorized={authorized} onChange={setAuthorized} />
          <ScanControls
            state={scan.state}
            authorized={authorized}
            onScan={handleScan}
            onCancel={scan.cancel}
          />
          {scan.error && (
            <div className="panel border-l-2 border-l-danger px-4 py-2 text-sm text-danger">
              {scan.error}
            </div>
          )}
          <Dashboard hosts={scan.hosts} />
          <ResultsTable hosts={scan.hosts} />
        </div>
      </main>
    </div>
  );
}
