import { useCallback, useRef, useState } from "react";
import type { Host, ScanOptions, ScanProgress, ScanResult } from "../types";
import { cancelScan, runScan } from "../lib/api";
import { compareIp } from "../lib/format";

export type ScanState = "idle" | "scanning" | "done" | "error";

export interface UseScan {
  state: ScanState;
  hosts: Host[];
  progress: ScanProgress;
  lastResult: ScanResult | null;
  error: string | null;
  start: (options: ScanOptions) => Promise<void>;
  cancel: () => Promise<void>;
  loadHosts: (hosts: Host[], result?: ScanResult | null) => void;
}

const EMPTY_PROGRESS: ScanProgress = { scanned: 0, total: 0, found: 0 };

export function useScan(): UseScan {
  const [state, setState] = useState<ScanState>("idle");
  const [hosts, setHosts] = useState<Host[]>([]);
  const [progress, setProgress] = useState<ScanProgress>(EMPTY_PROGRESS);
  const [lastResult, setLastResult] = useState<ScanResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Buffer incoming hosts so rapid event bursts don't thrash React state.
  const bufferRef = useRef<Host[]>([]);
  const flushRef = useRef<number | null>(null);

  const flush = useCallback(() => {
    if (bufferRef.current.length === 0) return;
    const incoming = bufferRef.current;
    bufferRef.current = [];
    setHosts((prev) => {
      const byIp = new Map(prev.map((h) => [h.ip, h]));
      for (const h of incoming) byIp.set(h.ip, h);
      return Array.from(byIp.values()).sort((a, b) => compareIp(a.ip, b.ip));
    });
  }, []);

  const start = useCallback(
    async (options: ScanOptions) => {
      setState("scanning");
      setHosts([]);
      setError(null);
      setProgress({ scanned: 0, total: 0, found: 0 });
      bufferRef.current = [];

      try {
        const result = await runScan(options, {
          onProgress: (p) => setProgress(p),
          onHost: (h) => {
            bufferRef.current.push(h);
            if (flushRef.current == null) {
              flushRef.current = window.setTimeout(() => {
                flushRef.current = null;
                flush();
              }, 80);
            }
          },
        });
        flush();
        // Authoritative final host list from the backend.
        setHosts([...result.hosts].sort((a, b) => compareIp(a.ip, b.ip)));
        setLastResult(result);
        setProgress((p) => ({ ...p, scanned: result.totalScanned, total: result.totalScanned }));
        setState("done");
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setState("error");
      }
    },
    [flush]
  );

  const cancel = useCallback(async () => {
    await cancelScan();
    flush();
    setState("done");
  }, [flush]);

  const loadHosts = useCallback((next: Host[], result: ScanResult | null = null) => {
    setHosts([...next].sort((a, b) => compareIp(a.ip, b.ip)));
    setLastResult(result);
    setState(next.length ? "done" : "idle");
    setError(null);
  }, []);

  return { state, hosts, progress, lastResult, error, start, cancel, loadHosts };
}
