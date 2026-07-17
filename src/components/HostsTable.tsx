import { useMemo, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  Check,
  Copy,
  Download,
  Globe,
  Monitor,
  Search,
  TerminalSquare,
} from "lucide-react";
import type { HostResult } from "../types";
import { api } from "../lib/api";
import { hasWeb, ipToNum, formatRelative, serviceLabel } from "../lib/format";

type SortKey = "ip" | "hostname" | "mac" | "vendor" | "ports" | "response" | "last_seen";
type SortDir = "asc" | "desc";

interface HostsTableProps {
  hosts: HostResult[];
  newIps: Set<string>;
  onExport: () => void;
}

const RISKY_PORTS = new Set([3389, 445]);

function PortBadges({ ports }: { ports: number[] }) {
  if (ports.length === 0) {
    return <span className="text-xs text-slate-600">—</span>;
  }
  return (
    <div className="flex flex-wrap gap-1">
      {ports.map((p) => {
        const risky = RISKY_PORTS.has(p);
        return (
          <span
            key={p}
            className={`chip ${
              risky
                ? "border-amber-500/30 bg-amber-500/10 text-amber-200"
                : "border-brand-400/25 bg-brand-500/10 text-brand-200"
            }`}
            title={`${p} · ${serviceLabel(p)}`}
          >
            {serviceLabel(p)}
          </span>
        );
      })}
    </div>
  );
}

function RowActions({ host }: { host: HostResult }) {
  const [copied, setCopied] = useState(false);
  const web = hasWeb(host.open_ports);
  const rdp = host.open_ports.includes(3389);
  const ssh = host.open_ports.includes(22);

  async function copy() {
    await api.copyIp(host.ip);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  }

  return (
    <div className="flex items-center justify-end gap-0.5 opacity-70 transition-opacity group-hover:opacity-100">
      <button className="btn-icon" title="Copy IP" onClick={copy}>
        {copied ? <Check className="h-4 w-4 text-brand-300" /> : <Copy className="h-4 w-4" />}
      </button>
      <button
        className={`btn-icon ${web ? "text-brand-300" : ""}`}
        title={web ? `Open web interface (:${web})` : "Open web interface"}
        onClick={() => api.openWeb(host.ip, web ?? undefined)}
      >
        <Globe className="h-4 w-4" />
      </button>
      <button
        className={`btn-icon ${rdp ? "text-amber-300" : ""}`}
        title="Open RDP"
        onClick={() => api.openRdp(host.ip)}
      >
        <Monitor className="h-4 w-4" />
      </button>
      <button
        className={`btn-icon ${ssh ? "text-brand-300" : ""}`}
        title="Open SSH"
        onClick={() => api.openSsh(host.ip)}
      >
        <TerminalSquare className="h-4 w-4" />
      </button>
    </div>
  );
}

export function HostsTable({ hosts, newIps, onExport }: HostsTableProps) {
  const [query, setQuery] = useState("");
  const [sortKey, setSortKey] = useState<SortKey>("ip");
  const [sortDir, setSortDir] = useState<SortDir>("asc");

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = q
      ? hosts.filter((h) => {
          const hay = [
            h.ip,
            h.hostname ?? "",
            h.mac ?? "",
            h.vendor ?? "",
            h.open_ports.join(" "),
            h.open_ports.map(serviceLabel).join(" "),
          ]
            .join(" ")
            .toLowerCase();
          return hay.includes(q);
        })
      : hosts.slice();

    rows.sort((a, b) => {
      let cmp = 0;
      switch (sortKey) {
        case "ip":
          cmp = ipToNum(a.ip) - ipToNum(b.ip);
          break;
        case "hostname":
          cmp = (a.hostname ?? "").localeCompare(b.hostname ?? "");
          break;
        case "mac":
          cmp = (a.mac ?? "").localeCompare(b.mac ?? "");
          break;
        case "vendor":
          cmp = (a.vendor ?? "").localeCompare(b.vendor ?? "");
          break;
        case "ports":
          cmp = a.open_ports.length - b.open_ports.length;
          break;
        case "response":
          cmp = (a.response_ms ?? Infinity) - (b.response_ms ?? Infinity);
          break;
        case "last_seen":
          cmp = a.last_seen.localeCompare(b.last_seen);
          break;
      }
      return sortDir === "asc" ? cmp : -cmp;
    });
    return rows;
  }, [hosts, query, sortKey, sortDir]);

  function toggleSort(key: SortKey) {
    if (key === sortKey) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      setSortDir("asc");
    }
  }

  const columns: Array<{ key: SortKey; label: string; className?: string }> = [
    { key: "ip", label: "IP address" },
    { key: "hostname", label: "Hostname" },
    { key: "mac", label: "MAC" },
    { key: "vendor", label: "Vendor" },
    { key: "ports", label: "Open ports" },
    { key: "response", label: "Resp", className: "text-right" },
    { key: "last_seen", label: "Last seen", className: "text-right" },
  ];

  return (
    <div className="panel flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-3 border-b border-white/5 p-3">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500" />
          <input
            className="input pl-9"
            placeholder="Filter by IP, hostname, MAC, vendor, or service…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            spellCheck={false}
          />
        </div>
        <div className="hidden text-xs text-slate-500 sm:block">
          {filtered.length} of {hosts.length}
        </div>
        <button className="btn-ghost" onClick={onExport} disabled={hosts.length === 0}>
          <Download className="h-4 w-4" />
          Export CSV
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full border-collapse text-sm">
          <thead className="sticky top-0 z-10 bg-arc-850/95 backdrop-blur">
            <tr className="text-left text-xs uppercase tracking-wide text-slate-500">
              {columns.map((c) => (
                <th
                  key={c.key}
                  className={`cursor-pointer select-none whitespace-nowrap px-3 py-2.5 font-medium hover:text-slate-300 ${c.className ?? ""}`}
                  onClick={() => toggleSort(c.key)}
                >
                  <span className={`inline-flex items-center gap-1 ${c.className === "text-right" ? "flex-row-reverse" : ""}`}>
                    {c.label}
                    <SortIcon active={sortKey === c.key} dir={sortDir} />
                  </span>
                </th>
              ))}
              <th className="px-3 py-2.5 text-right font-medium">Actions</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-white/5">
            {filtered.map((h) => (
              <tr key={h.ip} className="group transition-colors hover:bg-white/[0.03]">
                <td className="whitespace-nowrap px-3 py-2 font-mono text-slate-100">
                  <span className="inline-flex items-center gap-2">
                    {h.ip}
                    {newIps.has(h.ip) && (
                      <span className="chip border-brand-400/40 bg-brand-500/15 text-brand-200">new</span>
                    )}
                  </span>
                </td>
                <td className="max-w-[200px] truncate px-3 py-2 text-slate-300" title={h.hostname ?? ""}>
                  {h.hostname ?? <span className="text-slate-600">—</span>}
                </td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-xs text-slate-400">
                  {h.mac ?? <span className="text-slate-600">—</span>}
                </td>
                <td className="max-w-[220px] truncate px-3 py-2 text-slate-300" title={h.vendor ?? ""}>
                  {h.vendor ?? <span className="text-slate-600">Unknown</span>}
                </td>
                <td className="px-3 py-2">
                  <PortBadges ports={h.open_ports} />
                </td>
                <td className="whitespace-nowrap px-3 py-2 text-right font-mono text-xs text-slate-400">
                  {h.response_ms != null ? `${h.response_ms} ms` : "—"}
                </td>
                <td className="whitespace-nowrap px-3 py-2 text-right text-xs text-slate-500">
                  {formatRelative(h.last_seen)}
                </td>
                <td className="whitespace-nowrap px-3 py-2">
                  <RowActions host={h} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>

        {filtered.length === 0 && (
          <div className="flex flex-col items-center justify-center gap-1 py-16 text-center text-slate-500">
            <p className="text-sm">{hosts.length === 0 ? "No hosts yet." : "No matches for your filter."}</p>
            <p className="text-xs">
              {hosts.length === 0 ? "Run a scan to discover live devices." : "Try a different search term."}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

function SortIcon({ active, dir }: { active: boolean; dir: SortDir }) {
  if (!active) return <ArrowUpDown className="h-3 w-3 text-slate-600" />;
  return dir === "asc" ? (
    <ArrowUp className="h-3 w-3 text-brand-300" />
  ) : (
    <ArrowDown className="h-3 w-3 text-brand-300" />
  );
}
