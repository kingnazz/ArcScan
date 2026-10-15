// The Changes inbox.
//
// It answers "what changed, and what still needs my attention?" — which is what
// sets it apart from the Inventory (what exists) and History (what ran).
//
// A calm list of things that happened, not a queue with a backlog counter. Each
// entry says what changed, to which device, on which network and when, and
// offers only the actions that would do something. Related changes to one device
// in one scan are shown together, because a device that moved address and opened
// a port is one thing that happened.

import { forwardRef } from "react";
import { ArrowRightLeft, Check, MinusCircle, PlusCircle, RotateCcw } from "lucide-react";
import { Badge, Button, EmptyState, Select } from "../ui/primitives";
import { ExportMenu, SearchField, ViewHeader } from "../ui/patterns";
import { formatCount, formatDateTime, formatRelative, serviceWithPort } from "../lib/format";
import {
  CHANGE_VIEWS,
  CHANGE_WINDOWS,
  actionsFor,
  describeChange,
  groupChanges,
  type ChangeAction,
  type ChangeFilter,
  type ChangeGroup,
} from "../lib/changes";
import { CHANGE_TYPE_LABEL } from "../lib/export";
import type { ChangeEvent, ExportFormat, NetworkOption } from "../types";

export interface ChangesPanelProps {
  events: ChangeEvent[];
  /** Events before filtering, for the "no matches" state. */
  totalEvents: number;
  unreviewed: number;
  networks: NetworkOption[];
  loading: boolean;
  /** True when older events exist beyond the page that was loaded. */
  truncated: boolean;
  /**
   * The newest scan present when the database was upgraded. Non-zero means the
   * inbox legitimately starts empty on an upgraded install.
   */
  startsAfterScanId: number;
  filter: ChangeFilter;
  onFilterChange: (patch: Partial<ChangeFilter>) => void;
  onAction: (action: ChangeAction, event: ChangeEvent) => void;
  onAcknowledgeVisible: () => void;
  onExport: (format: ExportFormat) => void;
  exportOpen: boolean;
  onToggleExport: () => void;
  onCloseExport: () => void;
  exportScopeLabel: string;
  /** Open the source scan and its comparison. */
  onOpenScan: (scanId: number) => void;
  onStartScan: () => void;
}

export const ChangesPanel = forwardRef<HTMLInputElement, ChangesPanelProps>(function ChangesPanel(
  props,
  searchRef,
) {
  const { events, totalEvents, filter, onFilterChange, networks } = props;
  const groups = groupChanges(events);
  const multipleNetworks = networks.length > 1;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title="Changes"
        summary={
          props.loading
            ? undefined
            : `${props.unreviewed > 0 ? `${formatCount(props.unreviewed)} unreviewed` : "Nothing to review"}${
                events.length !== totalEvents ? ` · ${formatCount(events.length)} shown` : ""
              }`
        }
        purpose="What changed between scans of the same network, and what still needs a look. Review a change, then acknowledge it."
        actions={
          <>
            <Button
              size="sm"
              icon={<Check className="h-3.5 w-3.5" />}
              disabled={events.every((e) => e.state !== "unreviewed")}
              onClick={props.onAcknowledgeVisible}
              title="Acknowledge every unreviewed change currently shown"
            >
              Acknowledge visible
            </Button>
            <ExportMenu
              onExport={props.onExport}
              disabled={events.length === 0}
              open={props.exportOpen}
              onOpenChange={(next) => (next ? props.onToggleExport() : props.onCloseExport())}
              title="Export the changes shown (Ctrl or Cmd + E)"
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
          placeholder="Search changes"
          label="Search changes by device, address, network or service"
        />

        <Select
          aria-label="Filter changes"
          className="w-44 shrink-0"
          value={filter.view}
          onChange={(event) => onFilterChange({ view: event.target.value as ChangeFilter["view"] })}
        >
          {CHANGE_VIEWS.map((view) => (
            <option key={view.id} value={view.id}>
              {view.label}
            </option>
          ))}
        </Select>

        <Select
          aria-label="Filter changes by time"
          className="w-36 shrink-0"
          value={filter.window}
          onChange={(event) =>
            onFilterChange({ window: event.target.value as ChangeFilter["window"] })
          }
        >
          {CHANGE_WINDOWS.map((window) => (
            <option key={window.id} value={window.id}>
              {window.label}
            </option>
          ))}
        </Select>

        {multipleNetworks ? (
          <Select
            aria-label="Filter changes by network"
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
                {network.name}
              </option>
            ))}
          </Select>
        ) : null}
      </div>

      {groups.length === 0 ? (
        <EmptyState
          title={
            props.loading
              ? "Loading changes"
              : totalEvents === 0
                ? "No changes recorded yet"
                : "No changes match these filters"
          }
          description={
            props.loading
              ? undefined
              : totalEvents === 0
                ? props.startsAfterScanId > 0
                  ? "ArcScan started keeping this list when it upgraded to version 1.8. Changes appear here after your next completed scan. Earlier differences are still in each scan's comparison, under History."
                  : "Run two scans of the same network and ArcScan will list what changed between them here."
                : filter.view === "unreviewed"
                  ? "Nothing is waiting for review. Switch the filter to All changes to see what you have already acknowledged."
                  : "Try a shorter search term, a wider time range, or another network."
          }
          action={
            props.loading ? undefined : totalEvents === 0 ? (
              <Button onClick={props.onStartScan}>Go to Scan</Button>
            ) : (
              <Button
                onClick={() =>
                  onFilterChange({ query: "", view: "all", window: "all", networkId: null })
                }
              >
                Clear the filters
              </Button>
            )
          }
        />
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <ul className="divide-y divide-border">
            {groups.map((group) => (
              <ChangeGroupRow
                key={group.key}
                group={group}
                showNetwork={multipleNetworks}
                onAction={props.onAction}
                onOpenScan={props.onOpenScan}
              />
            ))}
          </ul>
          {props.truncated ? (
            <p className="px-3 py-2 text-xs text-text-muted">
              Showing the most recent changes. Older ones are kept and appear in an export.
            </p>
          ) : null}
        </div>
      )}
    </div>
  );
});

const ACTION_LABEL: Record<ChangeAction, string> = {
  review: "Review",
  trust: "Trust",
  rename: "Rename",
  ignore: "Ignore",
  acknowledge: "Acknowledge",
  reopen: "Reopen",
};

function ChangeGroupRow({
  group,
  showNetwork,
  onAction,
  onOpenScan,
}: {
  group: ChangeGroup;
  showNetwork: boolean;
  onAction: (action: ChangeAction, event: ChangeEvent) => void;
  onOpenScan: (scanId: number) => void;
}) {
  // The group's actions come from its first event; each event keeps its own row
  // and its own Acknowledge, so nothing is reviewed by accident.
  const lead = group.events[0];
  const actions = actionsFor(lead);

  return (
    <li className="flex gap-3 px-4 py-3 transition-colors duration-fast hover:bg-surface-hover">
      <KindMark event={lead} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <KindLabel event={lead} />
          <span className="text-[13px] font-semibold text-text">{group.deviceLabel}</span>
          {group.ip ? <span className="mono text-xs text-text-secondary">{group.ip}</span> : null}
          {showNetwork && group.networkName ? (
            <span className="text-xs text-text-muted">on {group.networkName}</span>
          ) : null}
          {lead.state === "acknowledged" ? <Badge>Acknowledged</Badge> : null}
          {lead.state === "ignored" ? <Badge>Ignored</Badge> : null}
          <span className="ml-auto shrink-0 text-xs text-text-muted" title={formatDateTime(group.at)}>
            {formatRelative(group.at)}
          </span>
        </div>

        <ul className="mt-1 space-y-0.5">
          {group.events.map((event) => (
            <li key={event.id} className="text-[13px] leading-relaxed text-text-secondary">
              <span className="text-text-muted">{CHANGE_TYPE_LABEL[event.change_type]}: </span>
              {event.change_type === "ports_changed" ? (
                <PortChange event={event} />
              ) : (
                <span className="mono">{describeChange(event)}</span>
              )}
            </li>
          ))}
        </ul>

        <div className="mt-2 flex flex-wrap items-center gap-1">
          {actions.map((action) => (
            <Button
              key={action}
              size="sm"
              variant={action === "review" ? "secondary" : "ghost"}
              onClick={() => onAction(action, lead)}
            >
              {ACTION_LABEL[action]}
            </Button>
          ))}
          {group.scanId != null ? (
            <button
              type="button"
              className="link-btn ml-1.5 !text-text-secondary"
              onClick={() => onOpenScan(group.scanId as number)}
              title="Open the scan that found this change, with its full comparison"
            >
              Open the scan
            </button>
          ) : (
            <span className="px-1.5 text-xs text-text-muted">
              The scan that found this has since been removed
            </span>
          )}
        </div>
      </div>
    </li>
  );
}

/** Opened and closed services, marked with a word as well as a colour. */
function PortChange({ event }: { event: ChangeEvent }) {
  if (event.opened_ports.length === 0 && event.closed_ports.length === 0) {
    return <span>Open services changed</span>;
  }
  return (
    <span className="inline-flex flex-wrap gap-x-3">
      {event.opened_ports.length > 0 ? (
        <span className="mono text-online">
          Opened: {event.opened_ports.map(serviceWithPort).join(", ")}
        </span>
      ) : null}
      {event.closed_ports.length > 0 ? (
        <span className="mono text-missing">
          Closed: {event.closed_ports.map(serviceWithPort).join(", ")}
        </span>
      ) : null}
    </span>
  );
}

const KIND: Record<
  "added" | "returned" | "missing" | "changed",
  { label: string; icon: React.ReactNode; tone: string; bg: string }
> = {
  added: {
    label: "New device",
    icon: <PlusCircle className="h-3.5 w-3.5" aria-hidden />,
    tone: "text-new",
    bg: "bg-[var(--new-subtle)]",
  },
  returned: {
    label: "Returned",
    icon: <RotateCcw className="h-3.5 w-3.5" aria-hidden />,
    tone: "text-accent-text",
    bg: "bg-accent-subtle",
  },
  missing: {
    label: "Missing",
    icon: <MinusCircle className="h-3.5 w-3.5" aria-hidden />,
    tone: "text-missing",
    bg: "bg-[var(--missing-subtle)]",
  },
  changed: {
    label: "Changed",
    icon: <ArrowRightLeft className="h-3.5 w-3.5" aria-hidden />,
    tone: "text-changed",
    bg: "bg-[var(--changed-subtle)]",
  },
};

function kindOf(event: ChangeEvent): keyof typeof KIND {
  switch (event.change_type) {
    case "device_added":
      return "added";
    case "device_returned":
      return "returned";
    case "device_missing":
      return "missing";
    default:
      return "changed";
  }
}

/** The kind of change as a tinted icon: the column the eye scans down. */
function KindMark({ event }: { event: ChangeEvent }) {
  const kind = KIND[kindOf(event)];
  return (
    <span
      className={`mt-px flex h-6 w-6 shrink-0 items-center justify-center rounded-md ${kind.bg} ${kind.tone}`}
      aria-hidden
    >
      {kind.icon}
    </span>
  );
}

/** The kind of change as a word, so it is never colour alone. */
function KindLabel({ event }: { event: ChangeEvent }) {
  const kind = KIND[kindOf(event)];
  return <span className={`text-xs font-semibold ${kind.tone}`}>{kind.label}</span>;
}
