import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import {
  applyComparison,
  isStaleEvent,
  removeHostByIp,
  rowsFromScanDetail,
  settleRows,
  upsertHost,
  type DeviceRow,
} from "../lib/live";
import type {
  HostEvent,
  HostRemovedEvent,
  ScanComparison,
  ScanDetail,
  ScanOptions,
  ScanProgress,
  ScanStarted,
} from "../types";

/** How the scan panel describes what it is showing. */
export type ScanMode = "idle" | "scanning" | "finished" | "history";

export interface ScanMeta {
  target: string;
  profile: string | null;
  scanId: number | null;
  /** Database id of the saved scan, once it has been written. */
  savedScanId: number | null;
  durationMs: number;
  scanned: number;
  probed: number;
  cancelled: boolean;
}

/** A queued event, applied in batches rather than one render at a time. */
type Queued =
  | { kind: "upsert"; host: HostEvent["host"]; pending: boolean }
  | { kind: "remove"; ip: string };

/** Roughly ten repaints a second: fast enough to feel live, cheap enough for a /16. */
const FLUSH_INTERVAL_MS = 100;

export interface UseLiveScanOptions {
  onError: (message: string, technical?: string) => void;
  /** Called after a scan is saved, so history can refresh. */
  onSaved?: (comparison: ScanComparison) => void;
  /** Scans to keep, applied after each save. */
  historyRetention: number;
}

/**
 * Runs scans and keeps the results table in step with them.
 *
 * Two things here are load-bearing. Events are queued and applied on an interval,
 * so a 65,000-address sweep cannot drive one React render per host. And every
 * event is checked against the scan the UI is currently showing, so a cancelled
 * scan winding down in the background cannot inject hosts into the next one.
 */
export function useLiveScan({ onError, onSaved, historyRetention }: UseLiveScanOptions) {
  const [rows, setRows] = useState<DeviceRow[]>([]);
  const [mode, setMode] = useState<ScanMode>("idle");
  const [stopping, setStopping] = useState(false);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [started, setStarted] = useState<ScanStarted | null>(null);
  const [comparison, setComparison] = useState<ScanComparison | null>(null);
  const [meta, setMeta] = useState<ScanMeta | null>(null);

  /** The scan whose events the UI accepts. */
  const activeScanId = useRef<number | null>(null);
  const queue = useRef<Queued[]>([]);
  const flushTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (flushTimer.current) clearTimeout(flushTimer.current);
    };
  }, []);

  const flush = useCallback(() => {
    flushTimer.current = null;
    const batch = queue.current;
    if (batch.length === 0) return;
    queue.current = [];
    setRows((current) => {
      let next = current;
      for (const item of batch) {
        next =
          item.kind === "upsert"
            ? upsertHost(next, item.host, item.pending)
            : removeHostByIp(next, item.ip);
      }
      return next;
    });
  }, []);

  const enqueue = useCallback(
    (item: Queued) => {
      queue.current.push(item);
      if (flushTimer.current == null) {
        flushTimer.current = setTimeout(flush, FLUSH_INTERVAL_MS);
      }
    },
    [flush],
  );

  const run = useCallback(
    async (opts: ScanOptions) => {
      // Claiming the slot before the first await means a second Scan click while
      // this one is starting cannot interleave two sets of events.
      activeScanId.current = null;
      queue.current = [];
      if (flushTimer.current) {
        clearTimeout(flushTimer.current);
        flushTimer.current = null;
      }

      setRows([]);
      setComparison(null);
      setProgress(null);
      setStarted(null);
      setStopping(false);
      setMode("scanning");
      setMeta({
        target: opts.target,
        profile: opts.profile,
        scanId: null,
        savedScanId: null,
        durationMs: 0,
        scanned: 0,
        probed: 0,
        cancelled: false,
      });

      try {
        const result = await api.scan(opts, {
          onStarted: (event: ScanStarted) => {
            activeScanId.current = event.scan_id;
            setStarted(event);
          },
          onProgress: (event: ScanProgress) => {
            if (isStaleEvent(event.scan_id, activeScanId.current)) return;
            setProgress(event);
          },
          onHostDiscovered: (event: HostEvent) => {
            if (isStaleEvent(event.scan_id, activeScanId.current)) return;
            // Still pending: the MAC, vendor and hostname arrive later.
            enqueue({ kind: "upsert", host: event.host, pending: true });
          },
          onHostUpdated: (event: HostEvent) => {
            if (isStaleEvent(event.scan_id, activeScanId.current)) return;
            enqueue({ kind: "upsert", host: event.host, pending: false });
          },
          onHostRemoved: (event: HostRemovedEvent) => {
            if (isStaleEvent(event.scan_id, activeScanId.current)) return;
            enqueue({ kind: "remove", ip: event.ip });
          },
        });

        flush();
        if (!mounted.current) return;

        setMode("finished");
        setStopping(false);
        setMeta({
          target: result.target,
          profile: result.profile,
          scanId: result.scan_id,
          savedScanId: null,
          durationMs: result.duration_ms,
          scanned: result.scanned,
          probed: result.probed,
          cancelled: result.cancelled,
        });
        // Settle immediately from the returned hosts so nothing is left looking
        // half-resolved if an update event was dropped.
        setRows((current) => settleRows(current));

        await persist(result, { onError, onSaved, historyRetention, setRows, setComparison, setMeta });
      } catch (error) {
        if (!mounted.current) return;
        activeScanId.current = null;
        setMode("idle");
        setStopping(false);
        setProgress(null);
        const message = error instanceof Error ? error.message : String(error);
        onError(message);
      }
    },
    [enqueue, flush, onError, onSaved, historyRetention],
  );

  const cancel = useCallback(async () => {
    if (mode !== "scanning" || stopping) return;
    setStopping(true);
    try {
      await api.cancelScan();
    } catch (error) {
      setStopping(false);
      onError(
        "ArcScan could not stop the scan.",
        error instanceof Error ? error.message : String(error),
      );
    }
  }, [mode, stopping, onError]);

  /** Show a scan reopened from history. */
  const showSavedScan = useCallback((detail: ScanDetail, diff: ScanComparison | null) => {
    activeScanId.current = null;
    queue.current = [];
    setMode("history");
    setStopping(false);
    setProgress(null);
    setStarted(null);
    setRows(applyComparison(rowsFromScanDetail(detail), diff));
    setComparison(diff);
    setMeta({
      target: detail.target,
      profile: detail.profile,
      scanId: null,
      savedScanId: detail.id,
      durationMs: detail.duration_ms,
      scanned: detail.scanned,
      probed: detail.probed,
      cancelled: detail.status === "cancelled",
    });
  }, []);

  /** Patch one row in place, after a device is renamed or reclassified. */
  const patchRow = useCallback((ip: string, patch: Partial<DeviceRow>) => {
    setRows((current) =>
      current.map((row) => (row.host.ip === ip ? { ...row, ...patch } : row)),
    );
  }, []);

  const reset = useCallback(() => {
    activeScanId.current = null;
    queue.current = [];
    setRows([]);
    setComparison(null);
    setProgress(null);
    setStarted(null);
    setMeta(null);
    setMode("idle");
  }, []);

  return {
    rows,
    mode,
    scanning: mode === "scanning",
    stopping,
    progress,
    started,
    comparison,
    meta,
    run,
    cancel,
    showSavedScan,
    patchRow,
    reset,
  };
}

/**
 * Save the scan, then rebuild the table from what was actually written.
 *
 * Reloading the saved scan rather than trusting the in-memory rows is what
 * guarantees the table matches history exactly, and it is also how the operator's
 * device names appear straight away.
 *
 * A save that fails does not discard the results: the scan already happened, and
 * losing it because a write failed would be the worse outcome.
 */
async function persist(
  result: Awaited<ReturnType<typeof api.scan>>,
  ctx: {
    onError: (message: string, technical?: string) => void;
    onSaved?: (comparison: ScanComparison) => void;
    historyRetention: number;
    setRows: React.Dispatch<React.SetStateAction<DeviceRow[]>>;
    setComparison: React.Dispatch<React.SetStateAction<ScanComparison | null>>;
    setMeta: React.Dispatch<React.SetStateAction<ScanMeta | null>>;
  },
): Promise<void> {
  try {
    const saved = await api.save(result);
    const detail = await api.getScan(saved.scan_id);
    ctx.setRows(applyComparison(rowsFromScanDetail(detail), saved.comparison));
    ctx.setComparison(saved.comparison);
    ctx.setMeta((current) => (current ? { ...current, savedScanId: saved.scan_id } : current));
    ctx.onSaved?.(saved.comparison);

    if (ctx.historyRetention > 0) {
      await api.pruneHistory(ctx.historyRetention);
    }
  } catch (error) {
    ctx.onError(
      "ArcScan finished the scan but could not save it to history. The results below are still complete.",
      error instanceof Error ? error.message : String(error),
    );
  }
}
