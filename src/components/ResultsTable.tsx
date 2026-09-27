// The results table.
//
// This is the surface the product is about, so the details matter: a sticky
// header, dense rows, one visual anchor per row (name, address, state), quieter
// treatment for supporting detail, and full keyboard navigation that does not
// close the device panel between steps.
//
// The state column carries information rather than a constant: every row in a
// scan answered, so a green dot on every row said nothing. The mark now shows
// whether the device is still being identified, and whether it is new, back or
// changed since the previous scan.

import { useEffect, useMemo, useRef } from "react";
import { CircleDot, Eye, ShieldCheck, Star } from "lucide-react";
import { Badge, StatusDot, type BadgeTone } from "../ui/primitives";
import { Tooltip } from "../ui/Popover";
import { SortHeader } from "../ui/patterns";
import {
  formatLatency,
  formatRelative,
  isSensitivePort,
  serviceInfo,
  serviceLabel,
  serviceWithPort,
} from "../lib/format";
import { rowName, tableName, type DeviceRow } from "../lib/live";
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
            {columns.map((column) => (
              <SortHeader
                key={column.key}
                label={column.label}
                active={sortKey === column.key}
                direction={sortDir}
                align={column.align}
                onSort={() => onSort(column.key)}
                width={column.key === "state" ? "3rem" : undefined}
              />
            ))}
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
}: {
  row: DeviceRow;
  columns: SortKey[];
  selected: boolean;
  onSelect: (ip: string) => void;
  onOpen: (ip: string) => void;
}) {
  const { host } = row;
  const show = (key: SortKey) => columns.includes(key);

  return (
    <tr
      data-ip={host.ip}
      // A double click opens the device; it should not also select a word.
      onMouseDown={(event) => {
        if (event.detail > 1) event.preventDefault();
      }}
      aria-selected={selected}
      onClick={() => onSelect(host.ip)}
      onDoubleClick={() => onOpen(host.ip)}
      // No entrance animation. Fading a row in animates the opacity of its text,
      // which puts it below the contrast floor while it runs, and a table that
      // moves under the pointer while results stream is worse than one that does
      // not. New rows simply appear.
      className="cursor-default"
    >
      {show("state") ? (
        <td className="!px-2">
          <StateCell row={row} />
        </td>
      ) : null}

      {show("name") ? (
        <td className="max-w-[16rem]">
          <NameCell row={row} />
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
 * The state column: one mark per row, always with a label.
 *
 * Still identifying (the MAC, vendor and name have not arrived), new, back,
 * changed, or simply answered. The change states are the ones worth a glance,
 * so they are the only ones in colour other than the plain "answered" green.
 */
function StateCell({ row }: { row: DeviceRow }) {
  if (row.pending) {
    return (
      <span className="flex items-center" title="Answered; still identifying">
        <CircleDot className="h-3 w-3 animate-pulse text-accent-text" aria-label="Answered; still identifying" />
      </span>
    );
  }
  const [tone, label]: [BadgeTone, string] =
    row.change === "new"
      ? ["new", "New since the previous scan"]
      : row.change === "returned"
        ? ["accent", "Returned since the previous scan"]
        : row.change === "changed"
          ? ["changed", "Changed since the previous scan"]
          : ["online", "Answered this scan"];
  return (
    <span className="flex items-center">
      <StatusDot tone={tone} label={label} />
    </span>
  );
}

/** How the operator has classified the device, as an icon with a name. */
function ClassificationIcon({ row }: { row: DeviceRow }) {
  switch (row.status) {
    case "trusted":
      return <ShieldCheck className="h-3.5 w-3.5 shrink-0 text-online" aria-label="Trusted" />;
    case "watched":
      return <Eye className="h-3.5 w-3.5 shrink-0 text-changed" aria-label="Watched" />;
    case "known":
      return <Star className="h-3.5 w-3.5 shrink-0 fill-current text-accent-text" aria-label="Known" />;
    default:
      return null;
  }
}

/**
 * The device's name, its classification, and what changed. A device with no
 * name of its own shows its manufacturer in a quieter tone, never its address
 * again: the address has its own column beside this one.
 */
function NameCell({ row }: { row: DeviceRow }) {
  const name = tableName(row);
  return (
    <div className="flex min-w-0 items-center gap-1.5">
      <ClassificationIcon row={row} />
      <span
        className={`truncate ${
          name.kind === "named"
            ? "font-medium text-text"
            : name.kind === "vendor"
              ? "text-text-secondary"
              : "italic text-text-muted"
        }`}
        title={rowName(row)}
      >
        {name.text}
      </span>
      {row.change === "new" ? <Badge tone="new">New</Badge> : null}
      {row.change === "returned" ? <Badge tone="accent">Returned</Badge> : null}
      {row.change === "changed" ? (
        <Tooltip content={<ChangeSummary row={row} />}>
          <Badge tone="changed">Changed</Badge>
        </Tooltip>
      ) : null}
    </div>
  );
}

/**
 * Services as name and number, comma separated. Sensitive ones are marked, but
 * everything stays plain text rather than becoming a wall of pills.
 *
 * A port with no known service shows only its number: printing "3000 · 3000"
 * would be noise dressed up as information.
 */
function ServiceList({ ports }: { ports: number[] }) {
  if (ports.length === 0) {
    return (
      <span className="text-text-muted" title="No probed port accepted a connection">
        No open ports
      </span>
    );
  }
  // Beyond five the list stops being readable at this width, so the remainder
  // becomes a count with the full set in the title.
  const shown = ports.slice(0, 5);
  const rest = ports.length - shown.length;

  return (
    <span className="mono block truncate" title={ports.map(serviceWithPort).join(", ")}>
      {shown.map((port, index) => {
        const name = serviceInfo(port)?.name;
        return (
          <span
            key={port}
            className={isSensitivePort(port) ? "text-warning" : "text-text-secondary"}
          >
            {index > 0 ? <span className="text-text-muted">, </span> : null}
            {name ? (
              <>
                <span className="font-medium">{name}</span>
                <span className="text-text-muted">&nbsp;·&nbsp;{port}</span>
              </>
            ) : (
              <span className="font-medium">{port}</span>
            )}
          </span>
        );
      })}
      {rest > 0 ? (
        <span className="text-text-muted">
          , +{rest} more
        </span>
      ) : null}
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
