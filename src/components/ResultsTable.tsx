// The results table.
//
// This is the surface the product is about, so the details matter: a sticky
// header, dense rows, one visual anchor per row (name, address, state), quieter
// treatment for supporting detail, and full keyboard navigation that does not
// close the device panel between steps.

import { useEffect, useMemo, useRef } from "react";
import { ArrowDown, ArrowUp, CircleDot, Eye, ShieldCheck, Sparkles, Star } from "lucide-react";
import { Badge, StatusDot } from "../ui/primitives";
import { Tooltip } from "../ui/Popover";
import {
  formatLatency,
  formatRelative,
  isSensitivePort,
  serviceLabel,
} from "../lib/format";
import { isUnnamed, rowName, type DeviceRow } from "../lib/live";
import { COLUMNS, type SortDir, type SortKey } from "../lib/table";
import type { Density } from "../lib/prefs";

export interface ResultsTableProps {
  rows: DeviceRow[];
  visibleColumns: SortKey[];
  sortKey: SortKey;
  sortDir: SortDir;
  onSort: (key: SortKey) => void;
  selectedIp: string | null;
  onSelect: (ip: string | null) => void;
  /** Enter or a double click opens the device panel. */
  onOpen: (ip: string) => void;
  density: Density;
  scanning: boolean;
}

export function ResultsTable({
  rows,
  visibleColumns,
  sortKey,
  sortDir,
  onSort,
  selectedIp,
  onSelect,
  onOpen,
  density,
  scanning,
}: ResultsTableProps) {
  const bodyRef = useRef<HTMLTableSectionElement>(null);
  const columns = useMemo(
    () => COLUMNS.filter((c) => visibleColumns.includes(c.key)),
    [visibleColumns],
  );

  // Keep the selected row on screen as the arrow keys move through the list.
  useEffect(() => {
    if (!selectedIp || !bodyRef.current) return;
    const row = bodyRef.current.querySelector<HTMLElement>(`[data-ip="${CSS.escape(selectedIp)}"]`);
    row?.scrollIntoView({ block: "nearest" });
  }, [selectedIp]);

  function onKeyDown(event: React.KeyboardEvent<HTMLTableSectionElement>) {
    if (rows.length === 0) return;
    const index = rows.findIndex((r) => r.host.ip === selectedIp);

    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const delta = event.key === "ArrowDown" ? 1 : -1;
      const next = index < 0 ? (delta > 0 ? 0 : rows.length - 1) : index + delta;
      const clamped = Math.min(rows.length - 1, Math.max(0, next));
      onSelect(rows[clamped].host.ip);
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      onSelect(rows[0].host.ip);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      onSelect(rows[rows.length - 1].host.ip);
      return;
    }
    if (event.key === "Enter" && index >= 0) {
      event.preventDefault();
      onOpen(rows[index].host.ip);
    }
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <table
        className={`data-table ${density === "comfortable" ? "density-comfortable" : ""}`}
        aria-rowcount={rows.length}
      >
        <thead>
          <tr>
            {columns.map((column) => {
              const active = sortKey === column.key;
              return (
                <th
                  key={column.key}
                  scope="col"
                  aria-sort={active ? (sortDir === "asc" ? "ascending" : "descending") : "none"}
                  className={column.align === "right" ? "text-right" : undefined}
                  style={column.key === "state" ? { width: "3.25rem" } : undefined}
                >
                  <button
                    type="button"
                    onClick={() => onSort(column.key)}
                    className={`inline-flex items-center gap-1 rounded transition-colors duration-fast hover:text-text ${
                      column.align === "right" ? "flex-row-reverse" : ""
                    } ${active ? "text-text" : ""}`}
                    title={`Sort by ${column.label.toLowerCase()}`}
                  >
                    {column.label}
                    {active ? (
                      sortDir === "asc" ? (
                        <ArrowUp className="h-3 w-3 text-accent-text" aria-hidden />
                      ) : (
                        <ArrowDown className="h-3 w-3 text-accent-text" aria-hidden />
                      )
                    ) : null}
                  </button>
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody
          ref={bodyRef}
          // One tab stop for the whole grid, with the arrow keys moving inside
          // it. Tabbing through 254 rows would be unusable.
          tabIndex={0}
          role="rowgroup"
          aria-label="Discovered devices"
          onKeyDown={onKeyDown}
          className="focus:outline-none"
        >
          {rows.map((row) => (
            <Row
              key={row.host.ip}
              row={row}
              columns={columns.map((c) => c.key)}
              selected={row.host.ip === selectedIp}
              onSelect={onSelect}
              onOpen={onOpen}
              animate={scanning}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Row({
  row,
  columns,
  selected,
  onSelect,
  onOpen,
  animate,
}: {
  row: DeviceRow;
  columns: SortKey[];
  selected: boolean;
  onSelect: (ip: string) => void;
  onOpen: (ip: string) => void;
  animate: boolean;
}) {
  const { host } = row;
  const show = (key: SortKey) => columns.includes(key);

  return (
    <tr
      data-ip={host.ip}
      aria-selected={selected}
      onClick={() => onSelect(host.ip)}
      onDoubleClick={() => onOpen(host.ip)}
      // Only new rows fade in, and only while a scan is running, so a settled
      // table never animates under the pointer.
      className={`cursor-default ${animate ? "animate-row-in" : ""}`}
    >
      {show("state") ? (
        <td className="!px-2">
          <StateCell row={row} />
        </td>
      ) : null}

      {show("name") ? (
        <td className="max-w-[15rem]">
          <div className="flex min-w-0 items-center gap-1.5">
            <span
              className={`truncate font-medium ${isUnnamed(row) ? "text-text-secondary" : "text-text"}`}
              title={rowName(row)}
            >
              {rowName(row)}
            </span>
            {row.change === "new" ? (
              <Badge tone="new" icon={<Sparkles className="h-2.5 w-2.5" aria-hidden />}>
                New
              </Badge>
            ) : null}
            {row.change === "returned" ? <Badge tone="accent">Back</Badge> : null}
            {row.change === "changed" ? (
              <Tooltip content={<ChangeSummary row={row} />}>
                <Badge tone="changed">Changed</Badge>
              </Tooltip>
            ) : null}
          </div>
        </td>
      ) : null}

      {show("ip") ? (
        <td className="mono text-text">{host.ip}</td>
      ) : null}

      {show("ports") ? (
        <td className="max-w-[20rem]">
          <ServiceList ports={host.open_ports} />
        </td>
      ) : null}

      {show("vendor") ? (
        <td className="max-w-[13rem] truncate text-text-secondary" title={host.vendor ?? ""}>
          {host.vendor ?? <span className="empty-value" />}
        </td>
      ) : null}

      {show("mac") ? (
        <td className="mono text-text-secondary">
          {host.mac ?? (row.pending ? <PendingMark /> : <span className="empty-value" />)}
        </td>
      ) : null}

      {show("os") ? (
        <td
          className="text-text-secondary"
          title={host.ttl != null ? `Guessed from TTL ${host.ttl}` : undefined}
        >
          {host.os_guess ?? <span className="empty-value" />}
        </td>
      ) : null}

      {show("response") ? (
        <td className="mono text-right text-text-secondary">
          <LatencyCell row={row} />
        </td>
      ) : null}

      {show("last_seen") ? (
        <td className="text-right text-text-muted" title={host.last_seen}>
          {formatRelative(host.last_seen)}
        </td>
      ) : null}
    </tr>
  );
}

/**
 * The state column: online, plus how the operator has classified the device.
 * Every indicator carries a label, so nothing here is communicated by colour
 * alone.
 */
function StateCell({ row }: { row: DeviceRow }) {
  const statusIcon = () => {
    switch (row.status) {
      case "trusted":
        return <ShieldCheck className="h-3.5 w-3.5 text-online" aria-label="Trusted" />;
      case "watched":
        return <Eye className="h-3.5 w-3.5 text-changed" aria-label="Watched" />;
      case "known":
        return <Star className="h-3.5 w-3.5 fill-current text-accent-text" aria-label="Known" />;
      default:
        return null;
    }
  };

  return (
    <span className="flex items-center gap-1.5">
      <StatusDot tone="online" label="Responded to this scan" />
      {statusIcon()}
    </span>
  );
}

/**
 * Services as name and number. Sensitive ones are marked, but everything stays
 * plain text rather than a wall of pills.
 */
function ServiceList({ ports }: { ports: number[] }) {
  if (ports.length === 0) {
    return (
      <span className="text-text-muted" title="No probed port accepted a connection">
        No open ports
      </span>
    );
  }
  // Beyond six the list stops being readable, so the rest is a count with the
  // full set in the title.
  const shown = ports.slice(0, 6);
  const rest = ports.length - shown.length;

  return (
    <span className="mono flex min-w-0 items-baseline gap-x-2 truncate" title={ports.join(", ")}>
      {shown.map((port) => (
        <span key={port} className={isSensitivePort(port) ? "text-warning" : "text-text-secondary"}>
          <span className="font-medium">{serviceLabel(port)}</span>
          <span className="text-text-muted"> · {port}</span>
        </span>
      ))}
      {rest > 0 ? <span className="text-text-muted">+{rest}</span> : null}
    </span>
  );
}

/**
 * Response time, with both measurements available on hover.
 *
 * The column is called Response rather than Ping because the value is the fastest
 * of the two, and calling a TCP handshake a ping would be wrong.
 */
function LatencyCell({ row }: { row: DeviceRow }) {
  const { icmp_ms, tcp_ms, response_ms } = row.host;
  const primary = formatLatency(icmp_ms ?? tcp_ms ?? response_ms);
  if (!primary) {
    return row.pending ? <PendingMark /> : <span className="empty-value" />;
  }

  const icmp = formatLatency(icmp_ms);
  const tcp = formatLatency(tcp_ms);
  return (
    <Tooltip
      side="top"
      content={
        <span className="block space-y-0.5">
          <span className="block">ICMP: {icmp ?? "no reply"}</span>
          <span className="block">TCP connect: {tcp ?? "no reply"}</span>
        </span>
      }
    >
      <span>{primary}</span>
    </Tooltip>
  );
}

/** A row is still being enriched; a dash here would read as "no value". */
function PendingMark() {
  return (
    <span className="inline-flex items-center text-text-muted" title="Still resolving">
      <CircleDot className="h-3 w-3 animate-pulse" aria-label="Still resolving" />
    </span>
  );
}

function ChangeSummary({ row }: { row: DeviceRow }) {
  if (row.changed_fields.length === 0) return <span>Changed since the previous scan</span>;
  return (
    <span className="block space-y-1">
      {row.changed_fields.map((field) => (
        <span key={field.field} className="block">
          <span className="font-semibold">{field.label}: </span>
          {field.field === "ports" ? (
            <span>
              {field.added_ports.length > 0
                ? `opened ${field.added_ports.map(serviceLabel).join(", ")}`
                : null}
              {field.added_ports.length > 0 && field.removed_ports.length > 0 ? "; " : null}
              {field.removed_ports.length > 0
                ? `closed ${field.removed_ports.map(serviceLabel).join(", ")}`
                : null}
            </span>
          ) : (
            <span>
              {field.from ?? "none"} to {field.to ?? "none"}
            </span>
          )}
        </span>
      ))}
    </span>
  );
}
