// The persistent Inventory.
//
// Every device ArcScan has recorded, across every scan, rather than the devices
// one scan happened to see. It shares the results table's density, keyboard
// behaviour and width-aware columns on purpose: the two tables answer different
// questions but should feel like the same tool.
//
// It opens with one line saying what it is for — what ArcScan believes exists
// — and a compact count, not a row of dashboard cards. This is a utility, and a
// device that went missing is more interesting than a large number.

import { forwardRef, useEffect, useMemo, useRef } from "react";
import { CheckCircle2, CircleHelp, MinusCircle, Send, ShieldCheck, StickyNote } from "lucide-react";
import { Badge, Button, EmptyState, Select } from "../ui/primitives";
import { ExportMenu, SearchField, SortHeader, ViewHeader } from "../ui/patterns";
import { formatCount, formatDateTime, formatLatency, formatRelative, serviceWithPort } from "../lib/format";
import {
  INVENTORY_COLUMNS,
  INVENTORY_VIEWS,
  PRESENCE_HINT,
  PRESENCE_LABEL,
  inventoryHeadline,
  type InventoryColumn,
  type InventoryFilter,
  type InventorySortKey,
  type SortDirection,
} from "../lib/inventory";
import { confidenceTone, deviceTypeLabel, sourcesLabel } from "../lib/discovery";
import { FRESHNESS_HINT, rowType } from "../lib/effectiveType";
import type { Density } from "../lib/prefs";
import type { ExportFormat, InventoryRow, NetworkOption, PresenceState } from "../types";

/** The bulk actions the selection toolbar offers. */
export type BulkAction = "trusted" | "unclassified" | "ignored" | "export" | "copy";

export interface InventoryPanelProps {
  rows: InventoryRow[];
  /** Rows before filtering, for the "no matches" state. */
  totalRows: number;
  counts: { present: number; missing: number; unknown: number };
  networks: NetworkOption[];
  /** True when no completed scan anywhere can decide presence. */
  needsCompletedScan: boolean;
  loading: boolean;
  filter: InventoryFilter;
  onFilterChange: (patch: Partial<InventoryFilter>) => void;
  sortKey: InventorySortKey;
  sortDir: SortDirection;
  onSort: (key: InventorySortKey) => void;
  visibleColumns: InventoryColumn[];
  density: Density;
  selectedId: number | null;
  onSelect: (id: number | null) => void;
  onOpen: (id: number) => void;
  selection: Set<number>;
  onSelectionChange: (next: Set<number>) => void;
  onBulkAction: (action: BulkAction) => void;
  onExport: (format: ExportFormat) => void;
  /** The export menu is owned by the caller so a hotkey can open it. */
  exportOpen: boolean;
  onToggleExport: () => void;
  onCloseExport: () => void;
  /** Says exactly what an export would contain, e.g. "Exports 9 selected devices". */
  exportScopeLabel: string;
  onStartScan: () => void;
  /** Device types present in the unfiltered set, for the type filter. */
  deviceTypes: string[];
  onSendToArcAtlas: () => void;
  sendToArcAtlasEnabled: boolean;
  sendToArcAtlasTitle: string;
}

export const InventoryPanel = forwardRef<HTMLInputElement, InventoryPanelProps>(
  function InventoryPanel(props, searchRef) {
    const {
      rows,
      totalRows,
      counts,
      networks,
      needsCompletedScan,
      loading,
      filter,
      onFilterChange,
      selection,
      onSelectionChange,
    } = props;

    const bodyRef = useRef<HTMLTableSectionElement>(null);
    const columns = useMemo(
      () => INVENTORY_COLUMNS.filter((c) => props.visibleColumns.includes(c.key)),
      [props.visibleColumns],
    );
    const multipleNetworks = networks.length > 1;

    // Keep the selected row on screen as the arrow keys move through the list.
    useEffect(() => {
      if (props.selectedId == null || !bodyRef.current) return;
      const row = bodyRef.current.querySelector<HTMLElement>(`[data-device="${props.selectedId}"]`);
      row?.scrollIntoView({ block: "nearest" });
    }, [props.selectedId]);

    function onKeyDown(event: React.KeyboardEvent<HTMLTableSectionElement>) {
      if (rows.length === 0) return;
      const index = rows.findIndex((r) => r.device_id === props.selectedId);

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const delta = event.key === "ArrowDown" ? 1 : -1;
        const next = index < 0 ? (delta > 0 ? 0 : rows.length - 1) : index + delta;
        props.onSelect(rows[Math.min(rows.length - 1, Math.max(0, next))].device_id);
        return;
      }
      if (event.key === "Home") {
        event.preventDefault();
        props.onSelect(rows[0].device_id);
        return;
      }
      if (event.key === "End") {
        event.preventDefault();
        props.onSelect(rows[rows.length - 1].device_id);
        return;
      }
      if (event.key === "Enter" && index >= 0) {
        event.preventDefault();
        props.onOpen(rows[index].device_id);
        return;
      }
      // Space toggles the checkbox for the focused row, so a selection can be
      // built without reaching for the mouse.
      if (event.key === " " && index >= 0) {
        event.preventDefault();
        toggle(rows[index].device_id);
      }
    }

    function toggle(id: number) {
      const next = new Set(selection);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      onSelectionChange(next);
    }

    const allShownSelected = rows.length > 0 && rows.every((r) => selection.has(r.device_id));

    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <ViewHeader
          title="Inventory"
          summary={
            loading && totalRows === 0
              ? undefined
              : rows.length === totalRows
                ? inventoryHeadline({ total: totalRows, ...counts })
                : `${formatCount(rows.length)} of ${formatCount(totalRows)} devices`
          }
          purpose="What ArcScan believes exists: every device it has recorded, across every scan and network."
          actions={
            <>
              <Button
                size="sm"
                icon={<Send className="h-3.5 w-3.5" />}
                disabled={!props.sendToArcAtlasEnabled || totalRows === 0}
                onClick={props.onSendToArcAtlas}
                title={props.sendToArcAtlasTitle}
              >
                Send to ArcAtlas
              </Button>
              <ExportMenu
                onExport={props.onExport}
                disabled={totalRows === 0}
                open={props.exportOpen}
                onOpenChange={(next) => (next ? props.onToggleExport() : props.onCloseExport())}
                title="Export the inventory (Ctrl or Cmd + E)"
                scope={props.exportScopeLabel}
              />
            </>
          }
        />

        <div className="toolbar">
          <SearchField
            ref={searchRef}
            value={filter.query}
            onChange={(query) => onFilterChange({ query })}
            placeholder="Search inventory"
            label="Search the inventory by name, address, MAC, manufacturer, service or note"
          />

          <Select
            aria-label="Filter the inventory"
            className="w-44 shrink-0"
            value={filter.view}
            onChange={(event) =>
              onFilterChange({ view: event.target.value as InventoryFilter["view"] })
            }
          >
            {/* Presence and classification are different questions, so the
                menu says which one each option answers. */}
            {INVENTORY_VIEWS.filter((view) => view.id === "all").map((view) => (
              <option key={view.id} value={view.id} title={view.hint}>
                {view.label}
              </option>
            ))}
            <optgroup label="In the latest scan">
              {INVENTORY_VIEWS.filter((view) => ["present", "missing", "unknown"].includes(view.id)).map(
                (view) => (
                  <option key={view.id} value={view.id} title={view.hint}>
                    {view.label}
                  </option>
                ),
              )}
            </optgroup>
            <optgroup label="Your classification">
              {INVENTORY_VIEWS.filter((view) => ["trusted", "unreviewed", "ignored"].includes(view.id)).map(
                (view) => (
                  <option key={view.id} value={view.id} title={view.hint}>
                    {view.label}
                  </option>
                ),
              )}
            </optgroup>
          </Select>

          {/* A network filter is noise for someone with one network. */}
          {multipleNetworks ? (
            <Select
              aria-label="Filter by network"
              className="w-44 shrink-0"
              value={filter.networkId ?? ""}
              onChange={(event) =>
                onFilterChange({
                  networkId: event.target.value === "" ? null : Number(event.target.value),
                })
              }
            >
              <option value="">All networks</option>
              {networks.map((network) => (
                <option key={network.id} value={network.id}>
                  {network.name} ({formatCount(network.device_count)})
                </option>
              ))}
            </Select>
          ) : null}

          {/* Offered only once discovery has actually typed something: a
              filter whose every option is Unknown is a control that does
              nothing. */}
          {props.deviceTypes.length > 1 ? (
            <Select
              aria-label="Filter by device type"
              className="w-40 shrink-0"
              value={filter.deviceType ?? ""}
              onChange={(event) =>
                onFilterChange({
                  deviceType: event.target.value === "" ? null : event.target.value,
                })
              }
            >
              <option value="">All types</option>
              {props.deviceTypes.map((type) => (
                <option key={type} value={type}>
                  {deviceTypeLabel(type)}
                </option>
              ))}
            </Select>
          ) : null}
        </div>

        {selection.size > 0 ? (
          <div
            role="toolbar"
            aria-label="Actions for the selected devices"
            className="flex flex-wrap items-center gap-1.5 border-b border-border bg-accent-subtle px-3 py-1.5"
          >
            <span className="mr-1 text-xs font-medium text-text">
              {formatCount(selection.size)}{" "}
              {selection.size === 1 ? "device selected" : "devices selected"}
            </span>
            <Button size="sm" variant="ghost" onClick={() => props.onBulkAction("trusted")}>
              Mark trusted
            </Button>
            <Button size="sm" variant="ghost" onClick={() => props.onBulkAction("unclassified")}>
              Mark not classified
            </Button>
            <Button size="sm" variant="ghost" onClick={() => props.onBulkAction("ignored")}>
              Ignore
            </Button>
            <Button size="sm" variant="ghost" onClick={() => props.onBulkAction("copy")}>
              Copy addresses
            </Button>
            <Button size="sm" variant="ghost" onClick={() => props.onBulkAction("export")}>
              Export selected
            </Button>
            <button
              type="button"
              className="link-btn ml-auto"
              onClick={() => onSelectionChange(new Set())}
            >
              Clear selection
            </button>
          </div>
        ) : null}

        {rows.length === 0 ? (
          totalRows === 0 ? (
            <EmptyState
              title={loading ? "Loading the inventory" : "No inventory yet"}
              description={
                loading
                  ? undefined
                  : "Run your first scan to start building an inventory. Every device ArcScan finds is kept here, with what you name it and what changed since."
              }
              action={
                loading ? undefined : <Button onClick={props.onStartScan}>Go to Scan</Button>
              }
            />
          ) : (
            <EmptyState
              title="No devices match these filters"
              description="Try a shorter search term, a different presence filter, or another network."
              action={
                <Button
                  onClick={() =>
                    onFilterChange({
                      query: "",
                      view: "all",
                      networkId: null,
                      deviceType: null,
                    })
                  }
                >
                  Clear the filters
                </Button>
              }
            />
          )
        ) : (
          <div className="min-h-0 flex-1 overflow-auto">
            {needsCompletedScan ? (
              <p
                role="status"
                className="border-b border-border bg-warning-subtle px-3 py-1.5 text-xs leading-relaxed text-warning"
              >
                No completed scan has run yet, so ArcScan cannot say which of these devices are
                still present. Run a scan to the end and presence appears here.
              </p>
            ) : null}
            <table
              className={`data-table ${props.density === "comfortable" ? "density-comfortable" : ""}`}
            >
              <thead>
                <tr>
                  <th scope="col" style={{ width: "2.25rem" }} className="!px-2">
                    <input
                      type="checkbox"
                      className="checkbox"
                      aria-label={
                        allShownSelected
                          ? "Clear the selection"
                          : "Select every device shown"
                      }
                      checked={allShownSelected}
                      onChange={() =>
                        onSelectionChange(
                          allShownSelected ? new Set() : new Set(rows.map((r) => r.device_id)),
                        )
                      }
                    />
                  </th>
                  {columns.map((column) => (
                    <SortHeader
                      key={column.key}
                      label={column.label}
                      active={props.sortKey === column.key}
                      direction={props.sortDir}
                      align={column.align}
                      onSort={() => props.onSort(column.key)}
                    />
                  ))}
                </tr>
              </thead>
              <tbody
                ref={bodyRef}
                // One tab stop for the whole grid, with the arrow keys moving
                // inside it, exactly as the scan results table behaves.
                tabIndex={0}
                aria-label="Inventory"
                onKeyDown={onKeyDown}
                className="focus:outline-none"
              >
                {rows.map((row) => (
                  <InventoryRowView
                    key={row.device_id}
                    row={row}
                    columns={columns.map((c) => c.key)}
                    selected={row.device_id === props.selectedId}
                    checked={selection.has(row.device_id)}
                    onToggle={() => toggle(row.device_id)}
                    onSelect={props.onSelect}
                    onOpen={props.onOpen}
                  />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    );
  },
);

function InventoryRowView({
  row,
  columns,
  selected,
  checked,
  onToggle,
  onSelect,
  onOpen,
}: {
  row: InventoryRow;
  columns: InventoryColumn[];
  selected: boolean;
  checked: boolean;
  onToggle: () => void;
  onSelect: (id: number) => void;
  onOpen: (id: number) => void;
}) {
  const show = (key: InventoryColumn) => columns.includes(key);

  return (
    <tr
      data-device={row.device_id}
      // A double click opens the device; it should not also select a word.
      onMouseDown={(event) => {
        if (event.detail > 1) event.preventDefault();
      }}
      aria-selected={selected}
      onClick={() => onSelect(row.device_id)}
      onDoubleClick={() => onOpen(row.device_id)}
      className="cursor-default"
    >
      <td className="!px-2">
        <input
          type="checkbox"
          className="checkbox"
          checked={checked}
          aria-label={`Select ${row.display_name}`}
          onClick={(event) => event.stopPropagation()}
          onChange={onToggle}
        />
      </td>

      {show("device") ? (
        <td className="max-w-[16rem]">
          <div className="flex min-w-0 items-center gap-1.5">
            <span
              className={`truncate font-medium ${row.custom_name ? "text-text" : "text-text-secondary"}`}
              title={row.display_name}
            >
              {row.display_name}
            </span>
            {row.notes_present ? (
              <StickyNote
                className="h-3 w-3 shrink-0 text-text-muted"
                aria-label="Has notes"
              />
            ) : null}
            {row.status === "trusted" ? (
              <ShieldCheck className="h-3.5 w-3.5 shrink-0 text-online" aria-label="Trusted" />
            ) : null}
            {row.status === "ignored" ? <Badge>Ignored</Badge> : null}
          </div>
        </td>
      ) : null}

      {show("address") ? <td className="mono text-text">{row.current_ip ?? "—"}</td> : null}

      {show("status") ? (
        <td>
          <PresenceCell presence={row.presence} />
        </td>
      ) : null}

      {show("services") ? (
        <td className="max-w-[18rem]">
          {row.open_ports.length === 0 ? (
            <span className="text-text-muted">No open ports</span>
          ) : (
            <span
              className="mono block truncate text-text-secondary"
              title={row.open_ports.map(serviceWithPort).join(", ")}
            >
              {row.open_ports.slice(0, 4).map(serviceWithPort).join(", ")}
              {row.open_ports.length > 4 ? `, +${row.open_ports.length - 4} more` : ""}
            </span>
          )}
        </td>
      ) : null}

      {show("manufacturer") ? (
        <td className="max-w-[13rem] truncate text-text-secondary" title={row.vendor ?? ""}>
          {row.vendor ?? <span className="empty-value" />}
        </td>
      ) : null}

      {show("network") ? (
        <td className="max-w-[10rem] truncate text-text-secondary">
          {row.network_name ?? <span className="empty-value" />}
        </td>
      ) : null}

      {show("last_seen") ? (
        <td className="text-right text-text-muted" title={formatDateTime(row.last_seen)}>
          {formatRelative(row.last_seen)}
        </td>
      ) : null}

      {show("mac") ? (
        <td className="mono text-text-secondary">
          {row.mac ?? <span className="empty-value" />}
        </td>
      ) : null}

      {show("hostname") ? (
        <td className="max-w-[12rem] truncate text-text-secondary">
          {row.hostname ?? <span className="empty-value" />}
        </td>
      ) : null}

      {show("first_seen") ? (
        <td className="text-right text-text-muted" title={formatDateTime(row.first_seen)}>
          {formatRelative(row.first_seen)}
        </td>
      ) : null}

      {show("observations") ? (
        <td className="mono text-right text-text-secondary">{row.observation_count}</td>
      ) : null}

      {show("response") ? (
        <td className="mono text-right text-text-secondary">
          {formatLatency(row.latest_icmp_ms ?? row.latest_tcp_ms ?? row.latest_response_ms) ?? (
            <span className="empty-value" />
          )}
        </td>
      ) : null}

      {show("previous") ? (
        <td className="mono text-text-secondary">
          {row.previous_ips[0] ?? <span className="empty-value" />}
        </td>
      ) : null}

      {show("type") ? (
        <td className="max-w-[11rem]">
          <TypeCell row={row} />
        </td>
      ) : null}

      {show("detected_name") ? (
        <td
          className="max-w-[14rem] truncate text-text-secondary"
          title={row.discovery?.detected_name ?? ""}
        >
          {row.discovery?.detected_name ?? <span className="empty-value" />}
        </td>
      ) : null}

      {show("model") ? (
        <td className="max-w-[12rem] truncate text-text-secondary" title={modelTitle(row)}>
          {modelTitle(row) || <span className="empty-value" />}
        </td>
      ) : null}

      {show("discovery_sources") ? (
        <td className="max-w-[10rem] truncate text-text-secondary">
          {row.discovery ? sourcesLabel(row.discovery.sources) : <span className="empty-value" />}
        </td>
      ) : null}

      {show("last_discovered") ? (
        <td className="text-right text-text-muted" title={row.discovery?.last_discovered_at ?? ""}>
          {row.discovery?.last_discovered_at ? (
            formatRelative(row.discovery.last_discovered_at)
          ) : (
            <span className="empty-value" />
          )}
        </td>
      ) : null}
    </tr>
  );
}

/** Manufacturer and model as one cell, without repeating the manufacturer. */
function modelTitle(row: InventoryRow): string {
  const make = row.discovery?.manufacturer?.trim() ?? "";
  const model = row.discovery?.model_name?.trim() ?? "";
  if (!model) return make;
  if (make && !model.toLowerCase().startsWith(make.toLowerCase())) return `${make} ${model}`;
  return model;
}

/**
 * Presence, as a word and an icon as well as a colour.
 *
 * Never "online": ArcScan does not watch the network, it reports what the last
 * completed scan found, and the tooltip says so in full.
 */
export function PresenceCell({ presence }: { presence: PresenceState }) {
  const icon =
    presence === "present" ? (
      <CheckCircle2 className="h-3.5 w-3.5 text-online" aria-hidden />
    ) : presence === "missing" ? (
      <MinusCircle className="h-3.5 w-3.5 text-missing" aria-hidden />
    ) : (
      <CircleHelp className="h-3.5 w-3.5 text-text-muted" aria-hidden />
    );
  return (
    <span className="inline-flex items-center gap-1.5" title={PRESENCE_HINT[presence]}>
      {icon}
      <span
        className={
          presence === "missing" ? "text-missing" : presence === "unknown" ? "text-text-muted" : ""
        }
      >
        {PRESENCE_LABEL[presence]}
      </span>
    </span>
  );
}

/**
 * The Type column.
 *
 * Shows the *effective* type — the operator's correction where there is one,
 * ArcScan's own answer otherwise — so the column, the filter and the export
 * cannot disagree. Which of the two it is has to be visible without being loud,
 * so a corrected type carries a quiet "You" rather than a confidence word: a
 * confidence badge beside a type a person chose would be ArcScan grading them.
 *
 * A type resting entirely on stale evidence is marked, because a column that
 * showed a three-scan-old answer identically to a fresh one would be the
 * clearest way to make discovery untrustworthy.
 */
function TypeCell({ row }: { row: InventoryRow }) {
  const resolved = rowType(row);
  if (resolved.effectiveType === "unknown" && !resolved.isUserSet) {
    return <span className="empty-value" />;
  }
  const freshness = row.discovery?.evidence_freshness;
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className="truncate">{deviceTypeLabel(resolved.effectiveType)}</span>
      {resolved.isUserSet ? (
        <Badge tone="accent" title="You set this device type. ArcScan's own answer is in the drawer.">
          You
        </Badge>
      ) : (
        <>
          <Badge tone={confidenceTone(resolved.detectedConfidence)}>
            {resolved.detectedConfidence}
          </Badge>
          {freshness === "stale" ? (
            <Badge tone="warning" title={FRESHNESS_HINT.stale}>
              stale
            </Badge>
          ) : null}
        </>
      )}
    </span>
  );
}
