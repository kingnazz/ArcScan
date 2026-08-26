// The application shell.
//
// Holds the state the panels share and nothing else: the scan itself lives in
// useLiveScan, the table's sorting and filtering in lib/table, device actions in
// lib/actions. Anything with logic worth testing is a pure function outside this
// file.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, DownloadCloud, X } from "lucide-react";
import { AppHeader, type View } from "./components/AppHeader";
import { CommandBar } from "./components/CommandBar";
import { ContextBar } from "./components/ContextBar";
import { ResultsToolbar } from "./components/ResultsToolbar";
import { ResultsTable } from "./components/ResultsTable";
import { DeviceDrawer } from "./components/DeviceDrawer";
import { HistoryPanel } from "./components/HistoryPanel";
import { ComparisonPanel } from "./components/ComparisonPanel";
import { InventoryPanel, type BulkAction } from "./components/InventoryPanel";
import { ChangesPanel } from "./components/ChangesPanel";
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
import { useRuntime } from "./hooks/useRuntime";
import { useUpdater } from "./hooks/useUpdater";
import { api } from "./lib/api";
import { setServiceCatalog } from "./lib/format";
import { rowFromInventory, rowName, rowsFromScanDetail } from "./lib/live";
import {
  markLegacyLabelsImported,
  pendingLegacyLabels,
  loadRecentTargets,
  pushRecentTarget,
} from "./lib/prefs";
import { recommendedProfile, type ProfileId } from "./lib/profiles";
import { EMPTY_FILTER, prepareRows, visibleColumns, type SortKey, type TableFilter } from "./lib/table";
import {
  EMPTY_INVENTORY_FILTER,
  prepareInventory,
  presentDeviceTypes,
  visibleInventoryColumns,
  type InventoryFilter,
  type InventorySortKey,
  type SortDirection,
} from "./lib/inventory";
import {
  EMPTY_CHANGE_FILTER,
  filterChanges,
  type ChangeAction,
  type ChangeFilter,
} from "./lib/changes";
import type { ActionId } from "./lib/actions";
import { type ChangeEvent, type ChangeFeed, type DeviceDetail, type DeviceStatus, type ExportFormat, type InventorySummary, type LocalNetwork, type NetworkScope, type ScanComparison, type ScanOptions, type ScanSummary } from "./types";
import { PORTABLE_UPDATE_STEPS } from "./lib/runtime";
import { APP_VERSION } from "./version";

/** Below this the drawer becomes an overlay rather than a second pane. */
const OVERLAY_BREAKPOINT = 1100;

export default function App() {
  const toast = useToast();
  const { settings, update: updateSettings, reset: resetSettings, loaded } = useSettings();
  const theme = useTheme(settings.theme);
  const runtime = useRuntime();
  const updater = useUpdater(settings.checkForUpdates, runtime?.updater_mode ?? "installer");
  const publicIp = usePublicIp();

  const [view, setView] = useState<View>("results");
  /**
   * Within the Scan view, the results table or this scan's comparison.
   *
   * The scan-to-scan comparison stays here rather than moving into Changes: it
   * describes one scan against one baseline, while Changes is the persistent
   * inbox across every scan. Collapsing the two would lose the detail.
   */
  const [scanTab, setScanTab] = useState<"devices" | "comparison">("devices");
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
  /** Which table the open drawer is describing. */
  const [drawerSource, setDrawerSource] = useState<"scan" | "inventory">("scan");
  const [deviceDetail, setDeviceDetail] = useState<DeviceDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [drawerWidth, setDrawerWidth] = useState(392);
  const [windowWidth, setWindowWidth] = useState(() =>
    typeof window === "undefined" ? 1440 : window.innerWidth,
  );
  const [pendingDelete, setPendingDelete] = useState<ScanSummary | null>(null);
  const [pendingBulk, setPendingBulk] = useState<{
    title: string;
    description: string;
    confirmLabel: string;
    run: () => void;
  } | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [scopes, setScopes] = useState<NetworkScope[]>([]);

  // --- Inventory ----------------------------------------------------------
  const [inventory, setInventory] = useState<InventorySummary | null>(null);
  const [inventoryLoading, setInventoryLoading] = useState(true);
  const [invFilter, setInvFilter] = useState<InventoryFilter>(EMPTY_INVENTORY_FILTER);
  const [invSortKey, setInvSortKey] = useState<InventorySortKey>("last_seen");
  const [invSortDir, setInvSortDir] = useState<SortDirection>("desc");
  const [invSelectedId, setInvSelectedId] = useState<number | null>(null);
  const [invSelection, setInvSelection] = useState<Set<number>>(() => new Set());
  const [invExportOpen, setInvExportOpen] = useState(false);
  const [highlightEventId, setHighlightEventId] = useState<number | null>(null);

  // --- Changes ------------------------------------------------------------
  const [changes, setChanges] = useState<ChangeFeed | null>(null);
  const [changesLoading, setChangesLoading] = useState(true);
  const [changeFilter, setChangeFilter] = useState<ChangeFilter>(EMPTY_CHANGE_FILTER);
  const [changeExportOpen, setChangeExportOpen] = useState(false);

  const targetInput = useRef<HTMLInputElement>(null);
  const filterInput = useRef<HTMLInputElement>(null);
  const inventorySearch = useRef<HTMLInputElement>(null);
  const changesSearch = useRef<HTMLInputElement>(null);

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

  /**
   * Reload the persistent inventory.
   *
   * Called after anything that can change it — a completed scan, a rename, a
   * status change, a network rename — rather than reloading the application, so
   * the scan in progress, the filters and the selection all survive.
   */
  const refreshInventory = useCallback(async () => {
    try {
      setInventory(await api.inventory());
    } catch (error) {
      const { message, technical } = describeError(error);
      reportError(`ArcScan could not read the device inventory. ${message}`, technical);
    } finally {
      setInventoryLoading(false);
    }
  }, [reportError]);

  const refreshChanges = useCallback(async () => {
    try {
      setChanges(await api.changeEvents());
    } catch (error) {
      const { message, technical } = describeError(error);
      reportError(`ArcScan could not read the list of changes. ${message}`, technical);
    } finally {
      setChangesLoading(false);
    }
  }, [reportError]);

  const scan = useLiveScan({
    onError: reportError,
    onSaved: (comparison) => {
      void refreshHistory();
      // A completed scan is exactly what moves presence and adds change events.
      void refreshInventory();
      void refreshChanges();
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
    void refreshInventory();
    void refreshChanges();
  }, [refreshHistory, refreshInventory, refreshChanges]);

  // Scopes appear in Settings; refresh them whenever the panel opens so a
  // just-created scope (first scan of a new network) is namable immediately.
  useEffect(() => {
    if (!settingsOpen) return;
    api
      .listNetworkScopes()
      .then(setScopes)
      .catch(() => {
        // Settings still works without the list; the section simply hides.
      });
  }, [settingsOpen]);

  const renameScope = useCallback(
    async (id: number, name: string) => {
      try {
        await api.renameNetworkScope(id, name);
        setScopes(await api.listNetworkScopes());
        // The name appears in History, the Inventory and the Changes inbox, so
        // all three are refreshed rather than left showing the old one.
        void refreshHistory();
        void refreshInventory();
        void refreshChanges();
        toast.success(`Network renamed to "${name}".`);
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not rename that network. ${message}`, technical);
      }
    },
    [refreshHistory, refreshInventory, refreshChanges, toast, reportError],
  );

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
  const overlayDrawers = windowWidth < OVERLAY_BREAKPOINT;

  const inventoryRows = useMemo(
    () => prepareInventory(inventory?.rows ?? [], invFilter, invSortKey, invSortDir),
    [inventory, invFilter, invSortKey, invSortDir],
  );
  const inventoryNetworks = inventory?.networks ?? [];
  // Computed from the unfiltered set, so choosing a type never removes the
  // other options from the menu that got you there.
  const inventoryDeviceTypes = useMemo(
    () => presentDeviceTypes(inventory?.rows ?? []),
    [inventory],
  );
  const inventoryColumns = useMemo(
    () =>
      visibleInventoryColumns(
        windowWidth,
        settings.inventoryColumns,
        inventoryNetworks.length > 1,
      ),
    [windowWidth, settings.inventoryColumns, inventoryNetworks.length],
  );
  const visibleChanges = useMemo(
    () => filterChanges(changes?.events ?? [], changeFilter),
    [changes, changeFilter],
  );

  // The drawer describes either a row of the current scan or a row of the
  // inventory. Resolving both to one shape keeps a single drawer rather than two
  // that would drift apart.
  const selectedRow = useMemo(() => {
    if (drawerSource === "inventory") {
      const row = inventory?.rows.find((r) => r.device_id === invSelectedId);
      return row ? rowFromInventory(row) : null;
    }
    return scan.rows.find((row) => row.host.ip === selectedIp) ?? null;
  }, [drawerSource, inventory, invSelectedId, scan.rows, selectedIp]);

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
      setScanTab("devices");
      setSelectedIp(null);
      setDrawerSource("scan");
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
        setDrawerSource("scan");
        setView("results");
        setScanTab("devices");
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
      setScanTab("comparison");
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

  // Export a historical scan by fetching it directly. Deliberately does NOT
  // route through the view state: React state updates are asynchronous, so
  // "open it, then export what is displayed" can silently export the previous
  // scan or the current filter. The current view, selection and filters are
  // left untouched, and a fetch failure exports nothing rather than falling
  // back to whatever is on screen.
  const exportSavedScan = useCallback(
    async (id: number, format: ExportFormat) => {
      try {
        const detail = await api.getScan(id);
        const exportRowsForScan = rowsFromScanDetail(detail);
        const written = await api.exportRows(exportRowsForScan, format, detail.target);
        if (written) {
          toast.success(
            `Exported ${exportRowsForScan.length} ${exportRowsForScan.length === 1 ? "device" : "devices"} from the scan of ${detail.target}.`,
          );
        }
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not export that scan. ${message}`, technical);
      }
    },
    [toast, reportError],
  );

  // --- Inventory and Changes ---------------------------------------------

  const openInventoryDevice = useCallback((deviceId: number, eventId: number | null = null) => {
    setInvSelectedId(deviceId);
    setDrawerSource("inventory");
    setHighlightEventId(eventId);
    setDrawerOpen(true);
  }, []);

  /** What an inventory export would contain, said plainly before it runs. */
  const inventoryExportScope = useMemo(() => {
    if (invSelection.size > 0) {
      return `Exports the ${formatPlural(invSelection.size, "selected device")}.`;
    }
    const network =
      invFilter.networkId == null
        ? null
        : inventoryNetworks.find((n) => n.id === invFilter.networkId)?.name;
    const filtered = inventoryRows.length !== (inventory?.rows.length ?? 0);
    if (network) return `Exports the ${formatPlural(inventoryRows.length, "device")} on ${network}.`;
    if (filtered) return `Exports the ${formatPlural(inventoryRows.length, "device")} shown.`;
    return `Exports all ${formatPlural(inventoryRows.length, "device")} across every network.`;
  }, [invSelection.size, invFilter.networkId, inventoryNetworks, inventoryRows.length, inventory]);

  const exportInventory = useCallback(
    async (format: ExportFormat) => {
      setInvExportOpen(false);
      const chosen =
        invSelection.size > 0
          ? inventoryRows.filter((row) => invSelection.has(row.device_id))
          : inventoryRows;
      if (chosen.length === 0) {
        toast.info("There is nothing to export yet.", {
          detail: "Run a scan, or clear the filters to include more devices.",
        });
        return;
      }
      const network =
        invFilter.networkId == null
          ? null
          : (inventoryNetworks.find((n) => n.id === invFilter.networkId)?.name ?? null);
      try {
        const written = await api.exportInventory(chosen, format, network);
        if (written) toast.success(`Exported ${formatPlural(chosen.length, "device")}.`);
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not write the export. ${message}`, technical);
      }
    },
    [inventoryRows, invSelection, invFilter.networkId, inventoryNetworks, toast, reportError],
  );

  const applyBulkStatus = useCallback(
    async (ids: number[], status: DeviceStatus, verb: string) => {
      try {
        const outcome = await api.setDeviceStatuses(ids, status);
        await refreshInventory();
        // The inbox hides an ignored device's changes, so it moves too.
        if (status === "ignored") await refreshChanges();
        if (outcome.missing.length > 0) {
          // Partial failure is reported rather than swallowed, and the selection
          // survives so the operator can see what is left.
          toast.info(
            `${verb} ${formatPlural(outcome.updated, "device")}. ${formatPlural(outcome.missing.length, "device")} could not be updated.`,
            { detail: "Those devices are no longer in the inventory." },
          );
          return;
        }
        toast.success(`${verb} ${formatPlural(outcome.updated, "device")}.`);
        setInvSelection(new Set());
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not update those devices. ${message}`, technical);
      }
    },
    [refreshInventory, refreshChanges, toast, reportError],
  );

  const runBulkAction = useCallback(
    (action: BulkAction) => {
      const ids = [...invSelection];
      if (ids.length === 0) return;
      const chosen = inventoryRows.filter((row) => invSelection.has(row.device_id));

      if (action === "copy") {
        const addresses = chosen.map((row) => row.current_ip).filter(Boolean).join("\n");
        void api.copyText(addresses);
        toast.success(`Copied ${formatPlural(chosen.length, "address", "addresses")}.`);
        return;
      }
      if (action === "export") {
        setInvExportOpen(true);
        return;
      }

      const label =
        action === "trusted" ? "Marked trusted" : action === "ignored" ? "Ignored" : "Marked unreviewed";
      // A large action confirms first; a handful of rows does not need a dialog.
      if (ids.length > 25) {
        setPendingBulk({
          title: `${action === "ignored" ? "Ignore" : "Update"} ${formatPlural(ids.length, "device")}?`,
          description:
            action === "ignored"
              ? "These devices stay in the inventory with all of their history. Their changes move out of the review inbox and can be filtered back in."
              : "This changes how these devices are classified. Nothing is removed.",
          confirmLabel: action === "ignored" ? "Ignore devices" : "Update devices",
          run: () => void applyBulkStatus(ids, action, label),
        });
        return;
      }
      void applyBulkStatus(ids, action, label);
    },
    [invSelection, inventoryRows, applyBulkStatus, toast],
  );

  const setChangeStates = useCallback(
    async (ids: number[], state: ChangeEvent["state"], message: string, undoTo?: ChangeEvent["state"]) => {
      try {
        const outcome = await api.setChangeState(ids, state);
        await refreshChanges();
        if (outcome.missing.length > 0) {
          toast.info(`${message} ${formatPlural(outcome.missing.length, "change")} was already gone.`);
          return;
        }
        toast.success(message, {
          onUndo:
            undoTo == null
              ? undefined
              : () => void setChangeStates(ids, undoTo, "Reopened.", undefined),
        });
      } catch (error) {
        const { message: text, technical } = describeError(error);
        reportError(`ArcScan could not update those changes. ${text}`, technical);
      }
    },
    [refreshChanges, toast, reportError],
  );

  const runChangeAction = useCallback(
    (action: ChangeAction, event: ChangeEvent) => {
      switch (action) {
        case "review":
        case "rename":
          if (event.device_id != null) openInventoryDevice(event.device_id, event.id);
          return;
        case "trust":
          if (event.device_id == null) return;
          void (async () => {
            await applyBulkStatus([event.device_id as number], "trusted", "Marked trusted");
            // Trust acknowledges the new-device entry it was offered on, and
            // nothing else about the device.
            await setChangeStates([event.id], "acknowledged", "Trusted and acknowledged.");
          })();
          return;
        case "ignore":
          if (event.device_id == null) return;
          void applyBulkStatus([event.device_id], "ignored", "Ignored");
          return;
        case "acknowledge":
          void setChangeStates([event.id], "acknowledged", "Acknowledged.", "unreviewed");
          return;
        case "reopen":
          void setChangeStates([event.id], "unreviewed", "Moved back to unreviewed.");
          return;
      }
    },
    [openInventoryDevice, applyBulkStatus, setChangeStates],
  );

  const acknowledgeVisible = useCallback(() => {
    // The captured set, not "whatever is visible when this finishes": a change
    // arriving mid-action must not be acknowledged without being seen.
    const ids = visibleChanges.filter((e) => e.state === "unreviewed").map((e) => e.id);
    if (ids.length === 0) return;
    const run = () =>
      void setChangeStates(
        ids,
        "acknowledged",
        `Acknowledged ${formatPlural(ids.length, "change")}.`,
        "unreviewed",
      );
    if (ids.length > 25) {
      setPendingBulk({
        title: `Acknowledge ${formatPlural(ids.length, "change")}?`,
        description:
          "Every unreviewed change currently shown is marked as reviewed. Nothing is deleted, and ignored records are left alone.",
        confirmLabel: "Acknowledge",
        run,
      });
      return;
    }
    run();
  }, [visibleChanges, setChangeStates]);

  const changesExportScope = useMemo(
    () => `Exports the ${formatPlural(visibleChanges.length, "change")} currently shown.`,
    [visibleChanges.length],
  );

  const exportChanges = useCallback(
    async (format: ExportFormat) => {
      setChangeExportOpen(false);
      if (visibleChanges.length === 0) return;
      const network =
        changeFilter.networkId == null
          ? null
          : (inventoryNetworks.find((n) => n.id === changeFilter.networkId)?.name ?? null);
      try {
        const written = await api.exportChanges(visibleChanges, format, network);
        if (written) toast.success(`Exported ${formatPlural(visibleChanges.length, "change")}.`);
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not write the export. ${message}`, technical);
      }
    },
    [visibleChanges, changeFilter.networkId, inventoryNetworks, toast, reportError],
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
        // A name appears in the Inventory and in the Changes inbox, so both are
        // refreshed rather than left showing the old one.
        void refreshInventory();
        void refreshChanges();
        toast.success(name ? `Renamed to "${name}".` : "Name cleared.", {
          onUndo: () => void renameDevice(deviceId, previous),
        });
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not save that name. ${message}`, technical);
      }
    },
    [scan, refreshInventory, refreshChanges, toast, reportError],
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
        void refreshInventory();
        if (status === "ignored" || previous === "ignored") void refreshChanges();
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
    [scan, refreshInventory, refreshChanges, toast, reportError],
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
        void refreshInventory();
        toast.success("Notes saved.");
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not save those notes. ${message}`, technical);
      }
    },
    [refreshInventory, toast, reportError],
  );

  /**
   * Correct, change or clear the detected device type.
   *
   * Rolls back on failure and says so, rather than leaving the drawer showing a
   * type the database does not hold. The Inventory is refreshed so the type
   * column and the type filter move together; nothing else is touched, because
   * a correction is a label and not an event.
   */
  const saveDeviceType = useCallback(
    async (deviceId: number, deviceType: string | null) => {
      try {
        await api.setDeviceTypeOverride(deviceId, deviceType);
        setDeviceDetail((current) =>
          current && current.device.id === deviceId
            ? { ...current, device: { ...current.device, user_device_type: deviceType } }
            : current,
        );
        void refreshInventory();
        toast.success(
          deviceType ? "Device type saved." : "Back to ArcScan's own answer.",
        );
        return true;
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not save that device type. ${message}`, technical);
        return false;
      }
    },
    [refreshInventory, toast, reportError],
  );

  /**
   * Put the redacted discovery report on the clipboard.
   *
   * Built where the data is — in Rust for the packaged app, in the demo backend
   * for the browser — so the interface never assembles it out of fields it
   * happens to have loaded, and so nothing it omits can be added back here by
   * accident.
   */
  const copyDiscoveryReport = useCallback(
    async (deviceId: number) => {
      try {
        const report = await api.deviceDiscoveryReport(deviceId);
        await api.copyText(report);
        toast.success("Discovery details copied. Nothing was sent anywhere.");
      } catch (error) {
        const { message, technical } = describeError(error);
        reportError(`ArcScan could not copy those discovery details. ${message}`, technical);
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
    // Then the current view's own state, before anything global. Clearing a
    // selection comes before clearing filters, because it is the more recent and
    // more surprising thing to leave behind.
    if (view === "inventory") {
      if (invSelection.size > 0) {
        setInvSelection(new Set());
        return;
      }
      if (
        invFilter.query ||
        invFilter.view !== "all" ||
        invFilter.networkId != null
      ) {
        setInvFilter(EMPTY_INVENTORY_FILTER);
        return;
      }
    } else if (view === "changes") {
      if (
        changeFilter.query ||
        changeFilter.window !== "all" ||
        changeFilter.networkId != null ||
        changeFilter.view !== EMPTY_CHANGE_FILTER.view
      ) {
        setChangeFilter(EMPTY_CHANGE_FILTER);
        return;
      }
    } else if (filter.query || filter.savedOnly || filter.changesOnly) {
      setFilter(EMPTY_FILTER);
      return;
    }
    if (scan.scanning) void scan.cancel();
  }, [settingsOpen, drawerOpen, view, invSelection, invFilter, changeFilter, filter, scan]);

  useHotkeys({
    onEscape,
    // Focus the search of whichever view is open rather than always jumping to
    // the scan results, which would throw away where the operator was.
    onFocusFilter: () => {
      const field =
        view === "inventory"
          ? inventorySearch.current
          : view === "changes"
            ? changesSearch.current
            : (setView("results"), filterInput.current);
      field?.focus();
      field?.select();
    },
    onExport: () => {
      if (view === "inventory") setInvExportOpen(true);
      else if (view === "changes") setChangeExportOpen(true);
      else void exportRows("csv");
    },
    onRescan: rescan,
    onFocusTarget: () => {
      targetInput.current?.focus();
      targetInput.current?.select();
    },
  });

  // --- Render -------------------------------------------------------------

  const showStart = scan.mode === "idle" && scan.rows.length === 0;
  const localCidrs = useMemo(() => localNetworks.map((n) => n.cidr), [localNetworks]);
  const copyPublicIp = useCallback(
    (ip: string) => {
      void api.copyText(ip);
      toast.success("Public IP copied.");
    },
    [toast],
  );
  const recommendFor = useCallback(
    (candidate: string) => recommendedProfile(candidate, localCidrs),
    [localCidrs],
  );

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <AppHeader
        view={view}
        onViewChange={setView}
        inventoryCount={inventory?.rows.length ?? 0}
        unreviewedChanges={changes?.unreviewed ?? 0}
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

      {/* Scan-screen context only: which network this machine is on, and the
          optional public-IP lookup. Inventory, Changes and History describe
          stored data rather than the current connection, so they do not pay the
          vertical space for it. */}
      {view === "results" ? (
        <ContextBar
          localNetworks={localNetworks}
          publicIp={publicIp.state}
          publicIpEnabled={settings.publicIpLookup}
          onCheckPublicIp={() => void publicIp.check()}
          onCopyPublicIp={copyPublicIp}
        />
      ) : null}

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
              onExport={(id, format) => void exportSavedScan(id, format)}
            />
          ) : view === "inventory" ? (
            <InventoryPanel
              ref={inventorySearch}
              rows={inventoryRows}
              totalRows={inventory?.rows.length ?? 0}
              counts={{
                present: inventory?.present ?? 0,
                missing: inventory?.missing ?? 0,
                unknown: inventory?.unknown ?? 0,
              }}
              networks={inventoryNetworks}
              needsCompletedScan={inventory?.needs_completed_scan ?? false}
              loading={inventoryLoading}
              filter={invFilter}
              onFilterChange={(patch) => setInvFilter((current) => ({ ...current, ...patch }))}
              sortKey={invSortKey}
              sortDir={invSortDir}
              onSort={(key) => {
                const nextDir = key === invSortKey && invSortDir === "asc" ? "desc" : "asc";
                setInvSortKey(key);
                setInvSortDir(nextDir);
              }}
              visibleColumns={inventoryColumns}
              density={settings.density}
              selectedId={invSelectedId}
              onSelect={setInvSelectedId}
              onOpen={(id) => openInventoryDevice(id)}
              selection={invSelection}
              onSelectionChange={setInvSelection}
              onBulkAction={runBulkAction}
              onExport={(format) => void exportInventory(format)}
              exportOpen={invExportOpen}
              onToggleExport={() => setInvExportOpen((v) => !v)}
              onCloseExport={() => setInvExportOpen(false)}
              exportScopeLabel={inventoryExportScope}
              onStartScan={() => setView("results")}
              deviceTypes={inventoryDeviceTypes}
            />
          ) : view === "changes" ? (
            <ChangesPanel
              ref={changesSearch}
              events={visibleChanges}
              totalEvents={changes?.total ?? 0}
              unreviewed={changes?.unreviewed ?? 0}
              networks={inventoryNetworks}
              loading={changesLoading}
              truncated={changes?.truncated ?? false}
              startsAfterScanId={changes?.starts_after_scan_id ?? 0}
              filter={changeFilter}
              onFilterChange={(patch) => setChangeFilter((current) => ({ ...current, ...patch }))}
              onAction={runChangeAction}
              onAcknowledgeVisible={acknowledgeVisible}
              onExport={(format) => void exportChanges(format)}
              exportOpen={changeExportOpen}
              onToggleExport={() => setChangeExportOpen((v) => !v)}
              onCloseExport={() => setChangeExportOpen(false)}
              exportScopeLabel={changesExportScope}
              onOpenScan={(id) => void compareSavedScan(id)}
              onStartScan={() => setView("results")}
            />
          ) : showStart ? (
            <ScanStart
              localNetworks={localNetworks}
              recommendFor={recommendFor}
              recents={recents}
              showGuidance={settings.showFirstRunGuidance}
              onScanNetwork={(cidr, profile) => {
                // Apply the recommendation the start screen displayed, so the
                // scan that runs is the scan that was described.
                setTarget(cidr);
                profileTouched.current = true;
                setProfileId(profile);
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
                onViewChanges={() => setScanTab((t) => (t === "comparison" ? "devices" : "comparison"))}
                comparisonOpen={scanTab === "comparison"}
                canExport={scan.rows.length > 0}
              />
              {scanTab === "comparison" ? (
                scan.comparison ? (
                  <ComparisonPanel
                    comparison={scan.comparison}
                    currentLabel={scan.meta?.target ?? target}
                    partial={scan.meta?.cancelled ?? false}
                    onBack={() => setScanTab("devices")}
                  />
                ) : (
                  <EmptyState
                    title="Nothing to compare yet"
                    description="ArcScan compares a scan with the most recent earlier completed scan that covered the same target with the same ports."
                    action={<Button onClick={() => setScanTab("devices")}>Back to devices</Button>}
                  />
                )
              ) : rows.length === 0 ? (
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
                    setDrawerSource("scan");
                    setHighlightEventId(null);
                    setDrawerOpen(true);
                  }}
                  density={settings.density}
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
          onTypeChange={(deviceId, deviceType) => saveDeviceType(deviceId, deviceType)}
          onCopyDiscovery={(deviceId) => void copyDiscoveryReport(deviceId)}
          scanKey={
            drawerSource === "inventory"
              ? "inventory"
              : (scan.meta?.savedScanId ?? scan.meta?.scanId ?? null)
          }
          context={drawerSource}
          highlightEventId={highlightEventId}
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
          runtime={runtime}
          onOpenDataFolder={() => {
            void api.openDataFolder().catch((e) => {
              const { message, technical } = describeError(e);
              reportError(message, technical);
            });
          }}
          publicIp={publicIp.state}
          onClearPublicIp={publicIp.clear}
          onOpenPrivacy={() => void api.openPrivacy()}
          scopes={scopes}
          onRenameScope={(id, name) => void renameScope(id, name)}
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

      <ConfirmDialog
        open={pendingBulk != null}
        title={pendingBulk?.title ?? ""}
        description={pendingBulk?.description ?? ""}
        confirmLabel={pendingBulk?.confirmLabel ?? "Continue"}
        onCancel={() => setPendingBulk(null)}
        onConfirm={() => {
          const action = pendingBulk;
          setPendingBulk(null);
          action?.run();
        }}
      />
    </div>
  );
}

/** `1 device` / `9 devices`, with an explicit plural for irregular words. */
function formatPlural(count: number, singular: string, plural?: string): string {
  const word = count === 1 ? singular : (plural ?? `${singular}s`);
  return `${count.toLocaleString()} ${word}`;
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
  const { status, version, progress, error, install, dismiss, mode } = updater;
  // Silence is the right response to "already up to date" on a background check.
  if (status === "idle" || status === "checking") return null;

  const busy = status === "downloading" || status === "installing";

  // Portable mode never offers to install. Replacing the application files in a
  // folder somebody chose, while keeping the ArcScanData beside them, is their
  // deliberate act -- so this says what to do and what to keep, and offers the
  // downloads page rather than an Update now button that would have nothing to
  // run: a portable build does not contain the installer updater at all.
  if (mode === "manual" && status === "available") {
    return (
      <div
        role="status"
        data-testid="portable-update-notice"
        className="animate-fade-in flex shrink-0 items-start gap-3 border-b border-border bg-accent-subtle px-3 py-2 text-[13px]"
      >
        <DownloadCloud className="mt-0.5 h-3.5 w-3.5 shrink-0 text-accent-text" aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="text-text">
            <span className="font-semibold">ArcScan {version}</span> is available.
          </p>
          <p className="mt-0.5 text-text-secondary">{PORTABLE_UPDATE_STEPS}</p>
        </div>
        <Button size="sm" variant="secondary" onClick={() => void api.openPortableDownloads()}>
          View portable downloads
        </Button>
        <button
          type="button"
          aria-label="Dismiss"
          onClick={dismiss}
          className="mt-0.5 shrink-0 rounded p-0.5 text-text-secondary hover:text-text"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
    );
  }

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
