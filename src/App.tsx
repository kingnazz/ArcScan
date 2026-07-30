// The application shell.
//
// Holds the state the panels share and nothing else: the scan itself lives in
// useLiveScan, the table's sorting and filtering in lib/table, device actions in
// lib/actions. Anything with logic worth testing is a pure function outside this
// file.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, DownloadCloud, X } from "lucide-react";
import { TitleBar, type View } from "./components/TitleBar";
import { CommandBar } from "./components/CommandBar";
import { ResultsToolbar } from "./components/ResultsToolbar";
import { ResultsTable } from "./components/ResultsTable";
import { DeviceDrawer } from "./components/DeviceDrawer";
import { HistoryPanel } from "./components/HistoryPanel";
import { ComparisonPanel } from "./components/ComparisonPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { ScanStart } from "./components/ScanStart";
import { ProgressStrip, StatusBar } from "./components/StatusBar";
import { Button, EmptyState } from "./ui/primitives";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { describeError, useToast } from "./ui/Toast";
import { useHotkeys } from "./hooks/useHotkeys";
import { useLiveScan } from "./hooks/useLiveScan";
import { usePublicIp } from "./hooks/usePublicIp";
import { useSettings } from "./hooks/useSettings";
import { useTheme } from "./hooks/useTheme";
import { useUpdater } from "./hooks/useUpdater";
import { api } from "./lib/api";
import { setServiceCatalog } from "./lib/format";
import { rowName } from "./lib/live";
import {
  markLegacyLabelsImported,
  pendingLegacyLabels,
  loadRecentTargets,
  pushRecentTarget,
} from "./lib/prefs";
import { recommendedProfile, type ProfileId } from "./lib/profiles";
import { EMPTY_FILTER, prepareRows, visibleColumns, type SortKey, type TableFilter } from "./lib/table";
import type { ActionId } from "./lib/actions";
import { changeCount, type DeviceDetail, type DeviceStatus, type ExportFormat, type LocalNetwork, type ScanComparison, type ScanOptions, type ScanSummary } from "./types";
import { APP_VERSION } from "./version";

const PRIVACY_URL = "https://kingnazz.github.io/ArcScan/privacy.html";
/** Below this the drawer becomes an overlay rather than a second pane. */
const OVERLAY_BREAKPOINT = 1100;

export default function App() {
  const toast = useToast();
  const { settings, update: updateSettings, reset: resetSettings, loaded } = useSettings();
  const theme = useTheme(settings.theme);
  const updater = useUpdater(settings.checkForUpdates);
  const publicIp = usePublicIp();

  const [view, setView] = useState<View>("results");
  const [target, setTarget] = useState("");
  const [profileId, setProfileId] = useState<ProfileId>("quick-lan");
  const [recents, setRecents] = useState<string[]>([]);
  const [localNetworks, setLocalNetworks] = useState<LocalNetwork[]>([]);
  const [history, setHistory] = useState<ScanSummary[]>([]);
  const [lastOptions, setLastOptions] = useState<ScanOptions | null>(null);

  const [filter, setFilter] = useState<TableFilter>(EMPTY_FILTER);
  const [sortKey, setSortKey] = useState<SortKey>("ip");
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc");

  const [selectedIp, setSelectedIp] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [deviceDetail, setDeviceDetail] = useState<DeviceDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [drawerWidth, setDrawerWidth] = useState(392);
  const [windowWidth, setWindowWidth] = useState(() =>
    typeof window === "undefined" ? 1440 : window.innerWidth,
  );
  const [pendingDelete, setPendingDelete] = useState<ScanSummary | null>(null);
  const [banner, setBanner] = useState<string | null>(null);

  const targetInput = useRef<HTMLInputElement>(null);
  const filterInput = useRef<HTMLInputElement>(null);

  const reportError = useCallback(
    (message: string, technical?: string) => {
      toast.error(message, { technical });
    },
    [toast],
  );

  const refreshHistory = useCallback(async () => {
    try {
      setHistory(await api.listScans());
    } catch (error) {
      const { message, technical } = describeError(error);
      reportError(`ArcScan could not read the scan history. ${message}`, technical);
    }
  }, [reportError]);

  const scan = useLiveScan({
    onError: reportError,
    onSaved: (comparison) => {
      void refreshHistory();
      if (settings.notifyOnChanges) announceChanges(comparison, toast.info);
    },
    historyRetention: settings.historyRetention,
  });

  // --- Startup ------------------------------------------------------------

  useEffect(() => {
    setRecents(loadRecentTargets());

    // The service table comes from the backend so the UI has no second copy.
    api
      .serviceCatalog()
      .then(setServiceCatalog)
      .catch(() => {
        // The built-in fallback covers the default profile's ports, so a failure
        // here degrades the labels rather than breaking the table.
      });

    api
      .detectNetworks()
      .then((networks) => {
        setLocalNetworks(networks);
        // Only prefill: never overwrite something already typed.
        setTarget((current) => current || networks[0]?.cidr || "");
      })
      .catch((error) => {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not detect this computer's networks. ${message}`, technical);
      });
  }, [reportError]);

  useEffect(() => {
    void refreshHistory();
  }, [refreshHistory]);

  // Adopt the device labels v1.6 kept in browser storage, once.
  useEffect(() => {
    const labels = pendingLegacyLabels();
    if (!labels) return;
    api
      .importDeviceLabels(labels)
      .then((adopted) => {
        markLegacyLabelsImported();
        if (adopted > 0) {
          toast.info(
            `Brought across ${adopted} device ${adopted === 1 ? "name" : "names"} from your previous version.`,
            { detail: "They are stored with the device inventory now, so they survive a reinstall." },
          );
        }
      })
      .catch(() => {
        // Left unmarked so the import is retried next launch. It only fills gaps,
        // so retrying can never overwrite a name set since.
      });
  }, [toast]);

  // The default profile applies until the operator picks one for this session.
  const profileTouched = useRef(false);
  useEffect(() => {
    if (loaded && !profileTouched.current) setProfileId(settings.defaultProfile);
  }, [loaded, settings.defaultProfile]);

  useEffect(() => {
    if (loaded) {
      setSortKey(settings.sortKey);
      setSortDir(settings.sortDir);
    }
    // Read once when settings arrive; afterwards the header owns the sort.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded]);

  useEffect(() => {
    function onResize() {
      setWindowWidth(window.innerWidth);
    }
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // --- Derived ------------------------------------------------------------

  const columns = useMemo(
    () => visibleColumns(windowWidth, settings.hiddenColumns),
    [windowWidth, settings.hiddenColumns],
  );
  const rows = useMemo(
    () => prepareRows(scan.rows, filter, sortKey, sortDir),
    [scan.rows, filter, sortKey, sortDir],
  );
  const selectedRow = useMemo(
    () => scan.rows.find((row) => row.host.ip === selectedIp) ?? null,
    [scan.rows, selectedIp],
  );
  const overlayDrawers = windowWidth < OVERLAY_BREAKPOINT;
  const totalChanges = changeCount(scan.comparison);

  // Load the device's stored history when the selection changes.
  useEffect(() => {
    const deviceId = selectedRow?.device_id;
    if (!drawerOpen || deviceId == null) {
      setDeviceDetail(null);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    api
      .deviceDetail(deviceId)
      .then((detail) => {
        if (!cancelled) setDeviceDetail(detail);
      })
      .catch((error) => {
        if (cancelled) return;
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not load this device's history. ${message}`, technical);
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [drawerOpen, selectedRow?.device_id, reportError]);

  // --- Actions ------------------------------------------------------------

  const runScan = useCallback(
    (opts: ScanOptions) => {
      setLastOptions(opts);
      setRecents(pushRecentTarget(opts.target));
      setView("results");
      setSelectedIp(null);
      setBanner(null);
      void scan.run(opts);
    },
    [scan],
  );

  const rescan = useCallback(() => {
    if (lastOptions && !scan.scanning) runScan(lastOptions);
  }, [lastOptions, scan.scanning, runScan]);

  const openSavedScan = useCallback(
    async (id: number) => {
      try {
        const [detail, comparison] = await Promise.all([
          api.getScan(id),
          api.compareScan(id).catch(() => null),
        ]);
        scan.showSavedScan(detail, comparison);
        setTarget(detail.target);
        setSelectedIp(null);
        setView("results");
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not open that scan. ${message}`, technical);
      }
    },
    [scan, reportError],
  );

  const compareSavedScan = useCallback(
    async (id: number) => {
      await openSavedScan(id);
      setView("changes");
    },
    [openSavedScan],
  );

  const deleteScan = useCallback(
    async (summary: ScanSummary) => {
      try {
        await api.deleteScan(summary.id);
        await refreshHistory();
        if (scan.meta?.savedScanId === summary.id) scan.reset();
        toast.success("Scan deleted.", {
          detail: "Device names, notes and first-seen dates were kept.",
        });
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not delete that scan. ${message}`, technical);
      }
    },
    [refreshHistory, scan, toast, reportError],
  );

  const exportRows = useCallback(
    async (format: ExportFormat) => {
      if (rows.length === 0) {
        toast.info("There is nothing to export yet.", {
          detail: "Run a scan, or clear the filter to include more devices.",
        });
        return;
      }
      try {
        const written = await api.exportRows(rows, format, scan.meta?.target ?? "scan");
        if (written) {
          toast.success(`Exported ${rows.length} ${rows.length === 1 ? "device" : "devices"}.`);
        }
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not write the export. ${message}`, technical);
      }
    },
    [rows, scan.meta?.target, toast, reportError],
  );

  const exportSavedScan = useCallback(
    async (id: number) => {
      await openSavedScan(id);
      await exportRows("csv");
    },
    [openSavedScan, exportRows],
  );

  const runDeviceAction = useCallback(
    async (id: ActionId, row: NonNullable<typeof selectedRow>, port?: number) => {
      const ip = row.host.ip;
      try {
        switch (id) {
          case "copy":
            await api.copyText(ip);
            toast.success(`Copied ${ip}.`);
            return;
          case "web":
            await api.openWeb(ip, port);
            return;
          case "smb":
            await api.openSmb(ip);
            return;
          case "rdp":
            await api.openRdp(ip);
            return;
          case "ssh":
            await api.openSsh(ip);
            return;
          case "wol":
            if (!row.host.mac) {
              toast.info("Wake-on-LAN needs a MAC address.", {
                detail:
                  "MAC addresses are only visible for devices on your local segment, so this is not available for routed targets.",
              });
              return;
            }
            await api.wakeOnLan(row.host.mac);
            toast.success(`Wake-on-LAN packet sent to ${rowName(row)}.`, {
              detail: "The device wakes only if Wake-on-LAN is enabled in its own settings.",
            });
            return;
        }
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`That action did not work for ${ip}. ${message}`, technical);
      }
    },
    [toast, reportError],
  );

  const renameDevice = useCallback(
    async (deviceId: number, name: string | null) => {
      const row = scan.rows.find((r) => r.device_id === deviceId);
      const previous = row?.custom_name ?? null;
      try {
        await api.setDeviceName(deviceId, name);
        if (row) scan.patchRow(row.host.ip, { custom_name: name });
        setDeviceDetail((current) =>
          current && current.device.id === deviceId
            ? { ...current, device: { ...current.device, custom_name: name } }
            : current,
        );
        toast.success(name ? `Renamed to "${name}".` : "Name cleared.", {
          onUndo: () => void renameDevice(deviceId, previous),
        });
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not save that name. ${message}`, technical);
      }
    },
    [scan, toast, reportError],
  );

  const changeDeviceStatus = useCallback(
    async (deviceId: number, status: DeviceStatus) => {
      const row = scan.rows.find((r) => r.device_id === deviceId);
      const previous = row?.status ?? "unclassified";
      try {
        await api.setDeviceStatus(deviceId, status);
        if (row) scan.patchRow(row.host.ip, { status });
        setDeviceDetail((current) =>
          current && current.device.id === deviceId
            ? { ...current, device: { ...current.device, status } }
            : current,
        );
        if (status !== previous) {
          toast.success(`Marked as ${status}.`, {
            onUndo: () => void changeDeviceStatus(deviceId, previous),
          });
        }
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not change that status. ${message}`, technical);
      }
    },
    [scan, toast, reportError],
  );

  const saveDeviceNotes = useCallback(
    async (deviceId: number, notes: string | null) => {
      try {
        await api.setDeviceNotes(deviceId, notes);
        setDeviceDetail((current) =>
          current && current.device.id === deviceId
            ? { ...current, device: { ...current.device, notes } }
            : current,
        );
        toast.success("Notes saved.");
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not save those notes. ${message}`, technical);
      }
    },
    [toast, reportError],
  );

  const toggleSort = useCallback(
    (key: SortKey) => {
      const nextDir = key === sortKey && sortDir === "asc" ? "desc" : "asc";
      setSortKey(key);
      setSortDir(nextDir);
      updateSettings({ sortKey: key, sortDir: nextDir });
    },
    [sortKey, sortDir, updateSettings],
  );

  // --- Keyboard -----------------------------------------------------------

  // Escape closes whatever is open, in the order it was opened, and only then
  // stops a scan. This keeps the key predictable: it never cancels a scan while
  // something is on top of the results.
  const onEscape = useCallback(() => {
    if (settingsOpen) {
      setSettingsOpen(false);
      return;
    }
    if (drawerOpen) {
      setDrawerOpen(false);
      return;
    }
    if (filter.query || filter.savedOnly || filter.changesOnly) {
      setFilter(EMPTY_FILTER);
      return;
    }
    if (scan.scanning) void scan.cancel();
  }, [settingsOpen, drawerOpen, filter, scan]);

  useHotkeys({
    onEscape,
    onFocusFilter: () => {
      setView("results");
      filterInput.current?.focus();
      filterInput.current?.select();
    },
    onExport: () => void exportRows("csv"),
    onRescan: rescan,
    onFocusTarget: () => {
      targetInput.current?.focus();
      targetInput.current?.select();
    },
  });

  // --- Render -------------------------------------------------------------

  const showStart = scan.mode === "idle" && scan.rows.length === 0;
  const recommended = recommendedProfile(
    target,
    localNetworks.map((n) => n.cidr),
  );

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <TitleBar
        view={view}
        onViewChange={setView}
        changeCount={totalChanges}
        hasComparison={scan.comparison != null}
        theme={theme}
        onToggleTheme={() => updateSettings({ theme: theme === "dark" ? "light" : "dark" })}
        onOpenSettings={() => setSettingsOpen((v) => !v)}
        settingsOpen={settingsOpen}
        onCheckUpdates={() => {
          if (api.native) void updater.check(true);
          else void api.openReleases();
        }}
        updateBusy={updater.status === "checking"}
      />

      <CommandBar
        ref={targetInput}
        scanning={scan.scanning}
        stopping={scan.stopping}
        target={target}
        onTargetChange={setTarget}
        profileId={profileId}
        onProfileChange={(id) => {
          profileTouched.current = true;
          setProfileId(id);
        }}
        settings={settings}
        onSettingsChange={updateSettings}
        recents={recents}
        localNetworks={localNetworks}
        canRescan={lastOptions != null}
        onScan={runScan}
        onStop={() => void scan.cancel()}
        onRescan={rescan}
        onError={(message) => toast.error(message)}
      />

      <ProgressStrip scanning={scan.scanning} progress={scan.progress} />

      {scan.started?.warning ? (
        <Notice
          tone="warning"
          onDismiss={() => setBanner(null)}
          message={scan.started.warning}
          hidden={banner === "dismissed"}
        />
      ) : null}

      <UpdateNotice updater={updater} />

      <div className="flex min-h-0 flex-1">
        <main className="flex min-w-0 flex-1 flex-col">
          {view === "history" ? (
            <HistoryPanel
              scans={history}
              activeId={scan.meta?.savedScanId ?? null}
              onOpen={(id) => void openSavedScan(id)}
              onCompare={(id) => void compareSavedScan(id)}
              onDelete={(id) => setPendingDelete(history.find((s) => s.id === id) ?? null)}
              onExport={(id) => void exportSavedScan(id)}
            />
          ) : view === "changes" ? (
            scan.comparison ? (
              <ComparisonPanel
                comparison={scan.comparison}
                currentLabel={scan.meta?.target ?? target}
              />
            ) : (
              <EmptyState
                title="No comparison yet"
                description="Run a scan, and ArcScan will compare it with the most recent earlier scan of the same target and profile."
                action={<Button onClick={() => setView("results")}>Back to devices</Button>}
              />
            )
          ) : showStart ? (
            <ScanStart
              localNetworks={localNetworks}
              recommended={recommended}
              recents={recents}
              showGuidance={settings.showFirstRunGuidance}
              onScanNetwork={(cidr) => {
                setTarget(cidr);
                // Deferred so the field shows the target before the scan starts.
                requestAnimationFrame(() => targetInput.current?.form?.requestSubmit());
              }}
              onPickTarget={(value) => {
                setTarget(value);
                targetInput.current?.focus();
              }}
              onDismissGuidance={() => updateSettings({ showFirstRunGuidance: false })}
            />
          ) : (
            <>
              <ResultsToolbar
                ref={filterInput}
                filter={filter}
                onFilterChange={(patch) => setFilter((current) => ({ ...current, ...patch }))}
                shown={rows.length}
                total={scan.rows.length}
                comparison={scan.comparison}
                onExport={(format) => void exportRows(format)}
                onViewChanges={() => setView("changes")}
                canExport={scan.rows.length > 0}
              />
              {rows.length === 0 ? (
                <EmptyState
                  title={
                    scan.rows.length === 0
                      ? scan.scanning
                        ? "Looking for devices"
                        : "No devices answered"
                      : "No devices match the filter"
                  }
                  description={
                    scan.rows.length === 0
                      ? scan.scanning
                        ? "Devices appear here as soon as they respond."
                        : "Nothing on this target replied to ICMP or TCP, and nothing appeared in the ARP table. Try the Reliable LAN profile, which waits longer and probes more ports."
                      : "Try a shorter search term, or clear the filter."
                  }
                  action={
                    scan.rows.length > 0 ? (
                      <Button onClick={() => setFilter(EMPTY_FILTER)}>Clear the filter</Button>
                    ) : undefined
                  }
                />
              ) : (
                <ResultsTable
                  rows={rows}
                  visibleColumns={columns}
                  sortKey={sortKey}
                  sortDir={sortDir}
                  onSort={toggleSort}
                  selectedIp={selectedIp}
                  onSelect={setSelectedIp}
                  onOpen={(ip) => {
                    setSelectedIp(ip);
                    setDrawerOpen(true);
                  }}
                  density={settings.density}
                  scanning={scan.scanning}
                />
              )}
            </>
          )}
        </main>

        <DeviceDrawer
          open={drawerOpen && selectedRow != null}
          row={selectedRow}
          detail={deviceDetail}
          loading={detailLoading}
          overlay={overlayDrawers}
          width={drawerWidth}
          onWidthChange={setDrawerWidth}
          onClose={() => setDrawerOpen(false)}
          onAction={(id, row, port) => void runDeviceAction(id, row, port)}
          onRename={(deviceId, name) => void renameDevice(deviceId, name)}
          onStatusChange={(deviceId, status) => void changeDeviceStatus(deviceId, status)}
          onNotesChange={(deviceId, notes) => void saveDeviceNotes(deviceId, notes)}
        />

        <SettingsPanel
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
          overlay={overlayDrawers}
          width={drawerWidth}
          onWidthChange={setDrawerWidth}
          settings={settings}
          onChange={updateSettings}
          onReset={() => {
            resetSettings();
            toast.success("Settings reset to their defaults.");
          }}
          version={APP_VERSION}
          native={api.native}
          publicIp={publicIp.state}
          onCheckPublicIp={() => void publicIp.check()}
          onClearPublicIp={publicIp.clear}
          onCopyPublicIp={(ip) => {
            void api.copyText(ip);
            toast.success("Public IP copied.");
          }}
          onOpenPrivacy={() => window.open(PRIVACY_URL, "_blank", "noopener")}
        />
      </div>

      <StatusBar
        mode={scan.mode}
        progress={scan.progress}
        meta={scan.meta}
        deviceCount={scan.rows.length}
        version={APP_VERSION}
        native={api.native}
      />

      <ConfirmDialog
        open={pendingDelete != null}
        title="Delete this scan?"
        description={
          <>
            The saved results for <span className="mono">{pendingDelete?.target}</span> from{" "}
            {pendingDelete ? new Date(pendingDelete.created_at).toLocaleString() : ""} are removed
            and cannot be recovered. Device names, notes and first-seen dates are kept.
          </>
        }
        confirmLabel="Delete scan"
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          const doomed = pendingDelete;
          setPendingDelete(null);
          if (doomed) void deleteScan(doomed);
        }}
      />
    </div>
  );
}

/** Summarise a comparison as one sentence, for the post-scan notification. */
function announceChanges(
  comparison: ScanComparison,
  notify: (message: string, options?: { detail?: string }) => void,
): void {
  const newCount = comparison.added.filter((d) => d.kind === "new").length;
  const returned = comparison.added.filter((d) => d.kind === "returned").length;
  const parts: string[] = [];
  if (newCount > 0) parts.push(`${newCount} new`);
  if (returned > 0) parts.push(`${returned} returned`);
  if (comparison.changed.length > 0) parts.push(`${comparison.changed.length} changed`);
  if (comparison.removed.length > 0) parts.push(`${comparison.removed.length} missing`);
  if (parts.length === 0) return;

  notify(`${parts.join(", ")} since the previous scan.`, {
    detail: "Open Changes to see the details.",
  });
}

function Notice({
  tone,
  message,
  onDismiss,
  hidden,
}: {
  tone: "warning" | "error";
  message: string;
  onDismiss: () => void;
  hidden: boolean;
}) {
  if (hidden) return null;
  return (
    <div
      role="status"
      className={`animate-fade-in flex shrink-0 items-start gap-2.5 border-b px-3 py-2 text-[13px] leading-relaxed ${
        tone === "warning"
          ? "border-warning/40 bg-warning-subtle text-warning"
          : "border-danger/40 bg-danger-subtle text-danger"
      }`}
    >
      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden />
      <p className="min-w-0 flex-1">{message}</p>
      <button
        type="button"
        aria-label="Dismiss"
        onClick={onDismiss}
        className="shrink-0 rounded p-0.5 hover:bg-black/5 dark:hover:bg-white/10"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

function UpdateNotice({ updater }: { updater: ReturnType<typeof useUpdater> }) {
  const { status, version, progress, error, install, dismiss } = updater;
  // Silence is the right response to "already up to date" on a background check.
  if (status === "idle" || status === "checking") return null;

  const busy = status === "downloading" || status === "installing";

  return (
    <div
      role="status"
      className="animate-fade-in flex shrink-0 items-center gap-3 border-b border-border bg-accent-subtle px-3 py-2 text-[13px]"
    >
      <DownloadCloud className="h-3.5 w-3.5 shrink-0 text-accent-text" aria-hidden />
      <p className="min-w-0 flex-1 text-text">
        {status === "available" ? (
          <>
            <span className="font-semibold">ArcScan {version}</span> is available.
          </>
        ) : status === "downloading" ? (
          <>Downloading the update{progress > 0 ? ` (${progress}%)` : ""}…</>
        ) : status === "installing" ? (
          "Installing. ArcScan will restart."
        ) : status === "uptodate" ? (
          <span className="text-text-secondary">ArcScan is up to date.</span>
        ) : (
          <span className="text-warning">
            The update check did not complete{error ? `: ${error}` : "."}
          </span>
        )}
      </p>
      {status === "available" ? (
        <Button size="sm" variant="primary" onClick={() => void install()}>
          Update now
        </Button>
      ) : null}
      {!busy ? (
        <button
          type="button"
          aria-label="Dismiss"
          onClick={dismiss}
          className="shrink-0 rounded p-0.5 text-text-secondary hover:text-text"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      ) : null}
    </div>
  );
}
