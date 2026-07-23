import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  Check,
  Copy,
  Download,
  FolderOpen,
  Globe,
  Monitor,
  Power,
  Search,
  Star,
  TerminalSquare,
} from "lucide-react";
import type { ExportFormat, HostResult } from "../types";
import { api } from "../lib/api";
import { hasWeb, ipToNum, formatRelative, serviceLabel } from "../lib/format";
import { isKnown, labelFor, type KnownMap } from "../lib/prefs";

type SortKey = "ip" | "hostname" | "mac" | "vendor" | "os" | "ports" | "response" | "last_seen";
type SortDir = "asc" | "desc";

interface HostsTableProps {
  hosts: HostResult[];
  newIps: Set<string>;
  onExport: (format: ExportFormat) => void;
  known: KnownMap;
  onToggleKnown: (mac: string, defaultLabel?: string) => void;
  onSetLabel: (mac: string, label: string) => void;
}

const RISKY_PORTS = new Set([23, 445, 3389, 5900]);

// Compact comma-separated service list; risky services highlighted.
function PortList({ ports }: { ports: number[] }) {
  if (ports.length === 0) {
    return <span className="text-faint">—</span>;
  }
  return (
    <span className="font-mono text-xs">
      {ports.map((p, i) => (
        <span key={p} title={`${p} · ${serviceLabel(p)}`}>
          {i > 0 && <span className="text-faint">, </span>}
          <span className={RISKY_PORTS.has(p) ? "text-amber-600 dark:text-amber-400" : "text-muted"}>
            {serviceLabel(p)}
          </span>
        </span>
      ))}
    </span>
  );
}

function RowActions({ host }: { host: HostResult }) {
  const [copied, setCopied] = useState(false);
  const web = hasWeb(host.open_ports);
  const rdp = host.open_ports.includes(3389);
  const ssh = host.open_ports.includes(22);
  const smb = host.open_ports.includes(445);

  async function copy() {
    await api.copyIp(host.ip);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  }

  return (
    <div className="flex items-center justify-end gap-0 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
      <button className="btn-icon" title="Copy IP" onClick={copy}>
        {copied ? <Check className="h-3.5 w-3.5 text-brand-600 dark:text-brand-300" /> : <Copy className="h-3.5 w-3.5" />}
      </button>
      <button
        className={`btn-icon ${web ? "text-brand-700 dark:text-brand-300" : ""}`}
        title={web ? `Open web interface (:${web})` : "Open web interface"}
        onClick={() => api.openWeb(host.ip, web ?? undefined)}
      >
        <Globe className="h-3.5 w-3.5" />
      </button>
      <button
        className={`btn-icon ${smb ? "text-brand-700 dark:text-brand-300" : ""}`}
        title="Open shared folders (SMB)"
        onClick={() => api.openSmb(host.ip)}
      >
        <FolderOpen className="h-3.5 w-3.5" />
      </button>
      <button
        className={`btn-icon ${rdp ? "text-amber-600 dark:text-amber-400" : ""}`}
        title="Open RDP"
        onClick={() => api.openRdp(host.ip)}
      >
        <Monitor className="h-3.5 w-3.5" />
      </button>
      <button
        className={`btn-icon ${ssh ? "text-brand-700 dark:text-brand-300" : ""}`}
        title="Open SSH"
        onClick={() => api.openSsh(host.ip)}
      >
        <TerminalSquare className="h-3.5 w-3.5" />
      </button>
      <button
        className="btn-icon disabled:opacity-30"
        title={host.mac ? "Wake-on-LAN (send magic packet)" : "Wake-on-LAN needs a known MAC"}
        onClick={() => host.mac && api.wakeOnLan(host.mac)}
        disabled={!host.mac}
      >
        <Power className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

export function HostsTable({
  hosts,
  newIps,
  onExport,
  known,
  onToggleKnown,
  onSetLabel,
}: HostsTableProps) {
  const [query, setQuery] = useState("");
  const [sortKey, setSortKey] = useState<SortKey>("ip");
  const [sortDir, setSortDir] = useState<SortDir>("asc");
  const [favOnly, setFavOnly] = useState(false);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    let rows = q
      ? hosts.filter((h) => {
          const hay = [
            h.ip,
            h.hostname ?? "",
            h.mac ?? "",
            h.vendor ?? "",
            h.os_guess ?? "",
            labelFor(known, h.mac),
            h.open_ports.join(" "),
            h.open_ports.map(serviceLabel).join(" "),
          ]
            .join(" ")
            .toLowerCase();
          return hay.includes(q);
        })
      : hosts.slice();

    if (favOnly) rows = rows.filter((h) => isKnown(known, h.mac));

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
        case "os":
          cmp = (a.os_guess ?? "").localeCompare(b.os_guess ?? "");
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
  }, [hosts, query, sortKey, sortDir, favOnly, known]);

  const knownCount = useMemo(
    () => hosts.filter((h) => isKnown(known, h.mac)).length,
    [hosts, known],
  );

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
    { key: "hostname", label: "Name" },
    { key: "mac", label: "MAC address" },
    { key: "vendor", label: "Manufacturer" },
    { key: "os", label: "OS" },
    { key: "ports", label: "Open ports" },
    { key: "response", label: "Ping", className: "text-right" },
    { key: "last_seen", label: "Last seen", className: "text-right" },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <div className="flex items-center gap-2 border-b border-line px-3 py-1.5">
        <div className="relative w-72 max-w-full">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-faint" />
          <input
            className="input pl-8"
            placeholder="Filter results…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            spellCheck={false}
          />
        </div>
        <div className="text-xs text-faint">
          {filtered.length === hosts.length ? `${hosts.length} hosts` : `${filtered.length} of ${hosts.length}`}
        </div>
        <div className="ml-auto flex items-center gap-2">
          <button
            className={`btn-ghost ${favOnly ? "border-amber-500/50 text-amber-600 dark:text-amber-400" : ""}`}
            onClick={() => setFavOnly((v) => !v)}
            disabled={knownCount === 0 && !favOnly}
            title="Show only saved/known devices"
          >
            <Star className={`h-3.5 w-3.5 ${favOnly ? "fill-current" : ""}`} />
            Saved{knownCount > 0 ? ` (${knownCount})` : ""}
          </button>
          <ExportMenu onExport={onExport} disabled={hosts.length === 0} />
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        <table className="grid-table">
          <thead>
            <tr>
              <th className="w-7 !px-2" aria-label="Status" />
              <th className="w-8 !px-1 text-center" aria-label="Saved" />
              {columns.map((c) => (
                <th
                  key={c.key}
                  className={`cursor-pointer select-none hover:text-fg ${c.className ?? ""}`}
                  onClick={() => toggleSort(c.key)}
                >
                  <span className={`inline-flex items-center gap-1 ${c.className === "text-right" ? "flex-row-reverse" : ""}`}>
                    {c.label}
                    {sortKey === c.key &&
                      (sortDir === "asc" ? (
                        <ArrowUp className="h-3 w-3 text-brand-600 dark:text-brand-300" />
                      ) : (
                        <ArrowDown className="h-3 w-3 text-brand-600 dark:text-brand-300" />
                      ))}
                  </span>
                </th>
              ))}
              <th className="text-right">Actions</th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((h) => {
              const saved = isKnown(known, h.mac);
              return (
                <tr key={h.ip} className="group">
                  <td className="!px-2 text-center">
                    <span
                      className="inline-block h-2 w-2 rounded-full bg-emerald-500"
                      title="Online"
                    />
                  </td>
                  <td className="!px-1 text-center">
                    <button
                      className={`btn-icon mx-auto h-6 w-6 disabled:opacity-25 ${
                        saved ? "text-amber-500 dark:text-amber-400" : "text-faint"
                      }`}
                      title={
                        !h.mac
                          ? "Saving needs a known MAC"
                          : saved
                            ? "Remove from saved devices"
                            : "Save this device"
                      }
                      disabled={!h.mac}
                      onClick={() => h.mac && onToggleKnown(h.mac, h.hostname ?? "")}
                    >
                      <Star className={`h-3.5 w-3.5 ${saved ? "fill-current" : ""}`} />
                    </button>
                  </td>
                  <td className="font-mono text-fg">
                    <span className="inline-flex items-center gap-2">
                      {h.ip}
                      {newIps.has(h.ip) && (
                        <span className="chip border-brand-500/40 bg-brand-500/15 text-brand-700 dark:text-brand-300">
                          new
                        </span>
                      )}
                    </span>
                  </td>
                  <td className="max-w-[220px] !whitespace-normal text-muted" title={h.hostname ?? ""}>
                    {saved && h.mac ? (
                      <input
                        className="w-full rounded-sm border border-line bg-surface px-1.5 py-0.5 text-xs text-fg placeholder:text-faint focus:border-brand-500 focus:outline-none"
                        value={labelFor(known, h.mac)}
                        placeholder={h.hostname ?? "Label this device…"}
                        onChange={(e) => onSetLabel(h.mac!, e.target.value)}
                        spellCheck={false}
                      />
                    ) : (
                      <span className="block truncate">{h.hostname ?? <span className="text-faint">—</span>}</span>
                    )}
                  </td>
                  <td className="font-mono text-xs text-muted">{h.mac ?? <span className="text-faint">—</span>}</td>
                  <td className="max-w-[220px] truncate text-muted" title={h.vendor ?? ""}>
                    {h.vendor ?? <span className="text-faint">Unknown</span>}
                  </td>
                  <td className="text-muted" title={h.ttl != null ? `TTL ${h.ttl}` : ""}>
                    {h.os_guess ?? <span className="text-faint">—</span>}
                  </td>
                  <td className="max-w-[280px] truncate">
                    <PortList ports={h.open_ports} />
                  </td>
                  <td className="text-right font-mono text-xs text-muted">
                    {h.response_ms != null ? `${h.response_ms} ms` : "—"}
                  </td>
                  <td className="text-right text-xs text-faint">{formatRelative(h.last_seen)}</td>
                  <td className="!py-0">
                    <RowActions host={h} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>

        {filtered.length === 0 && (
          <div className="flex flex-col items-center justify-center gap-1 py-16 text-center text-muted">
            <p className="text-sm">{hosts.length === 0 ? "No hosts yet." : "No matches for your filter."}</p>
            <p className="text-xs text-faint">
              {hosts.length === 0 ? "Run a scan to discover live devices." : "Try a different search term."}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

function ExportMenu({
  onExport,
  disabled,
}: {
  onExport: (format: ExportFormat) => void;
  disabled: boolean;
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

  const formats: Array<{ key: ExportFormat; label: string }> = [
    { key: "csv", label: "CSV (.csv)" },
    { key: "json", label: "JSON (.json)" },
    { key: "xml", label: "XML (.xml)" },
  ];

  return (
    <div className="relative" ref={ref}>
      <button className="btn-ghost" onClick={() => setOpen((v) => !v)} disabled={disabled}>
        <Download className="h-3.5 w-3.5" />
        Export
      </button>
      {open && (
        <div className="absolute right-0 z-20 mt-1 w-40 overflow-hidden rounded-md border border-line bg-surface shadow-panel animate-fade-in">
          {formats.map((f) => (
            <button
              key={f.key}
              className="flex w-full items-center px-3 py-1.5 text-left text-[13px] text-fg hover:bg-surface2"
              onClick={() => {
                setOpen(false);
                onExport(f.key);
              }}
            >
              {f.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
