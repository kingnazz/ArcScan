import { useMemo, useState } from "react";
import {
  Check,
  Copy,
  Globe,
  Monitor,
  Search,
  Terminal,
  ArrowUpDown,
} from "lucide-react";
import type { Host } from "../types";
import { copyToClipboard, launchAction } from "../lib/api";
import { compareIp, formatRelative } from "../lib/format";

interface ResultsTableProps {
  hosts: Host[];
}

type SortKey = "ip" | "hostname" | "vendor" | "responseMs" | "ports";
type SortDir = "asc" | "desc";

const PORT_TONE: Record<number, string> = {
  3389: "bg-danger/15 text-danger",
  445: "bg-danger/15 text-danger",
  22: "bg-accent/15 text-accent-soft",
  80: "bg-base-700 text-slate-300",
  443: "bg-ok/15 text-ok",
  8080: "bg-base-700 text-slate-300",
};

function CopyButton({ value, title }: { value: string; title: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="icon-btn"
      title={title}
      onClick={async () => {
        await copyToClipboard(value);
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      }}
    >
      {copied ? <Check className="h-4 w-4 text-ok" /> : <Copy className="h-4 w-4" />}
    </button>
  );
}

export function ResultsTable({ hosts }: ResultsTableProps) {
  const [query, setQuery] = useState("");
  const [sortKey, setSortKey] = useState<SortKey>("ip");
  const [sortDir, setSortDir] = useState<SortDir>("asc");

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = q
      ? hosts.filter((h) =>
          [h.ip, h.hostname, h.mac, h.vendor, h.openPorts.map((p) => `${p.port} ${p.service}`).join(" ")]
            .filter(Boolean)
            .some((field) => String(field).toLowerCase().includes(q))
        )
      : hosts;

    const sorted = [...rows].sort((a, b) => {
      let cmp = 0;
      switch (sortKey) {
        case "ip":
          cmp = compareIp(a.ip, b.ip);
          break;
        case "hostname":
          cmp = (a.hostname ?? "").localeCompare(b.hostname ?? "");
          break;
        case "vendor":
          cmp = (a.vendor ?? "").localeCompare(b.vendor ?? "");
          break;
        case "responseMs":
          cmp = (a.responseMs ?? Infinity) - (b.responseMs ?? Infinity);
          break;
        case "ports":
          cmp = a.openPorts.length - b.openPorts.length;
          break;
      }
      return sortDir === "asc" ? cmp : -cmp;
    });
    return sorted;
  }, [hosts, query, sortKey, sortDir]);

  const toggleSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      setSortDir("asc");
    }
  };

  const Th = ({ label, k, className = "" }: { label: string; k?: SortKey; className?: string }) => (
    <th
      className={`text-left font-medium text-xs uppercase tracking-wide text-slate-400 px-3 py-2 ${
        k ? "cursor-pointer hover:text-slate-200 select-none" : ""
      } ${className}`}
      onClick={k ? () => toggleSort(k) : undefined}
    >
      <span className="inline-flex items-center gap-1">
        {label}
        {k && (
          <ArrowUpDown
            className={`h-3 w-3 ${sortKey === k ? "text-accent-soft" : "text-slate-600"}`}
          />
        )}
      </span>
    </th>
  );

  return (
    <div className="panel flex flex-col min-h-0 flex-1 overflow-hidden">
      <div className="flex items-center gap-3 px-3 py-2.5 border-b border-base-700/70">
        <div className="relative flex-1 max-w-xs">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-slate-500" />
          <input
            className="input w-full pl-8 py-1.5 text-xs"
            placeholder="Filter by IP, host, MAC, vendor, port…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <span className="text-xs text-slate-500 tabular-nums">
          {filtered.length} of {hosts.length} hosts
        </span>
      </div>

      <div className="overflow-auto flex-1">
        <table className="w-full border-collapse text-sm">
          <thead className="sticky top-0 z-10 bg-base-850/95 backdrop-blur">
            <tr className="border-b border-base-700">
              <th className="w-8 px-3 py-2" />
              <Th label="IP Address" k="ip" />
              <Th label="Hostname" k="hostname" />
              <Th label="MAC" />
              <Th label="Vendor" k="vendor" />
              <Th label="Open Ports" k="ports" />
              <Th label="RTT" k="responseMs" />
              <Th label="Last Seen" />
              <th className="px-3 py-2 text-right text-xs uppercase tracking-wide text-slate-400">
                Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((h) => (
              <HostRow key={h.ip} host={h} />
            ))}
            {filtered.length === 0 && (
              <tr>
                <td colSpan={9} className="px-3 py-16 text-center text-sm text-slate-500">
                  No hosts to display.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function HostRow({ host }: { host: Host }) {
  const hasPort = (p: number) => host.openPorts.some((x) => x.port === p);
  const webPort = [443, 80, 8080].find((p) => hasPort(p));

  return (
    <tr className="border-b border-base-800/70 hover:bg-base-800/40 transition-colors group">
      <td className="px-3 py-2">
        <span
          className={`block h-2 w-2 rounded-full ${
            host.status === "up" ? "bg-ok animate-pulse-ring" : "bg-base-500"
          }`}
          title={host.status}
        />
      </td>
      <td className="px-3 py-2 font-mono text-slate-100 selectable whitespace-nowrap">
        {host.ip}
        {host.isNew && (
          <span className="chip ml-2 bg-ok/15 text-ok align-middle">new</span>
        )}
      </td>
      <td className="px-3 py-2 text-slate-300 max-w-[14rem] truncate selectable">
        {host.hostname ?? <span className="text-slate-600">—</span>}
      </td>
      <td className="px-3 py-2 font-mono text-xs text-slate-400 selectable whitespace-nowrap">
        {host.mac ?? <span className="text-slate-600">—</span>}
      </td>
      <td className="px-3 py-2 text-slate-400 max-w-[12rem] truncate selectable">
        {host.vendor ?? <span className="text-slate-600">—</span>}
      </td>
      <td className="px-3 py-2">
        <div className="flex flex-wrap gap-1">
          {host.openPorts.length === 0 && <span className="text-slate-600 text-xs">—</span>}
          {host.openPorts.map((p) => (
            <span
              key={p.port}
              className={`chip ${PORT_TONE[p.port] ?? "bg-base-700 text-slate-300"}`}
              title={p.service}
            >
              {p.port}
            </span>
          ))}
        </div>
      </td>
      <td className="px-3 py-2 tabular-nums text-slate-400 whitespace-nowrap">
        {host.responseMs != null ? `${host.responseMs} ms` : "—"}
      </td>
      <td className="px-3 py-2 text-slate-500 text-xs whitespace-nowrap">
        {formatRelative(host.lastSeen)}
      </td>
      <td className="px-3 py-2">
        <div className="flex items-center justify-end gap-0.5 opacity-60 group-hover:opacity-100 transition-opacity">
          <CopyButton value={host.ip} title="Copy IP" />
          <button
            className="icon-btn"
            title="Open web interface"
            disabled={!webPort}
            onClick={() => webPort && launchAction("web", host.ip, webPort)}
          >
            <Globe className="h-4 w-4" />
          </button>
          <button
            className="icon-btn"
            title="Open RDP"
            disabled={!hasPort(3389)}
            onClick={() => launchAction("rdp", host.ip, 3389)}
          >
            <Monitor className="h-4 w-4" />
          </button>
          <button
            className="icon-btn"
            title="Open SSH"
            disabled={!hasPort(22)}
            onClick={() => launchAction("ssh", host.ip, 22)}
          >
            <Terminal className="h-4 w-4" />
          </button>
        </div>
      </td>
    </tr>
  );
}
