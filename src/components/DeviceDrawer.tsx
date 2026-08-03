// The device detail panel.
//
// Typography and dividers rather than a card per section, so the panel reads as
// one document. Actions are always in the same place and in the same order, but
// only the ones the open services support are enabled, and the one the operator
// most likely wants is emphasised.

import { useEffect, useMemo, useState } from "react";
import {
  Copy,
  Eye,
  EyeOff,
  FolderOpen,
  Globe,
  Monitor,
  Power,
  ShieldCheck,
  Star,
  TerminalSquare,
} from "lucide-react";
import { Badge, Button, DetailRow, SectionHeading, Select } from "../ui/primitives";
import { Drawer } from "../ui/Drawer";
import {
  formatDateTime,
  formatLatency,
  formatRelative,
  isSensitivePort,
  serviceWithPort,
} from "../lib/format";
import { deviceActions, primaryAction, type ActionId } from "../lib/actions";
import { EMPTY_DRAFT, notesFromDetail, reconcileDraft } from "../lib/drawerDraft";
import { PRESENCE_HINT } from "../lib/inventory";
import { describeChange } from "../lib/changes";
import { CHANGE_TYPE_LABEL } from "../lib/export";
import { rowName, type DeviceRow } from "../lib/live";
import type { ChangeEvent, DeviceDetail, DeviceStatus, FieldChange } from "../types";

const STATUS_LABEL: Record<DeviceStatus, string> = {
  unclassified: "Not classified",
  known: "Known",
  trusted: "Trusted",
  watched: "Watched",
  ignored: "Ignored",
};

const STATUS_HINT: Record<DeviceStatus, string> = {
  unclassified: "Seen on the network, never labelled.",
  known: "Recognised and expected here.",
  trusted: "Recognised, and its open services are deliberate.",
  watched: "Recognised, and worth a look whenever it changes.",
  ignored: "Kept in the inventory, with its changes out of the review inbox.",
};

export interface DeviceDrawerProps {
  open: boolean;
  row: DeviceRow | null;
  detail: DeviceDetail | null;
  loading: boolean;
  overlay: boolean;
  width: number;
  onWidthChange: (width: number) => void;
  onClose: () => void;
  onAction: (id: ActionId, row: DeviceRow, port?: number) => void;
  onRename: (deviceId: number, name: string | null) => void;
  onStatusChange: (deviceId: number, status: DeviceStatus) => void;
  onNotesChange: (deviceId: number, notes: string | null) => void;
  /** Identity of the scan being shown, scoping drafts for rows not yet saved. */
  scanKey: number | string | null;
  /**
   * Where the drawer was opened from.
   *
   * `scan` shows the device as one scan saw it, so it can honestly say the
   * device answered. `inventory` shows the persistent record, where the only
   * truthful statement about presence is what the latest completed scan found.
   */
  context?: "scan" | "inventory";
  /** A change event to call out first, when the drawer was opened from Changes. */
  highlightEventId?: number | null;
}

export function DeviceDrawer({
  open,
  row,
  detail,
  loading,
  overlay,
  width,
  onWidthChange,
  onClose,
  onAction,
  onRename,
  onStatusChange,
  onNotesChange,
  scanKey,
  context = "scan",
  highlightEventId = null,
}: DeviceDrawerProps) {
  // Drafts are keyed by persistent device id (see lib/drawerDraft): they
  // survive IP changes, renames and incoming enrichment, reset when the panel
  // moves to a different device, and are never overwritten while dirty — so a
  // failed save keeps what the operator typed.
  const [draft, setDraft] = useState(EMPTY_DRAFT);
  useEffect(() => {
    setDraft((current) => reconcileDraft(current, row, detail, scanKey));
  }, [row, detail, scanKey]);

  const actions = useMemo(() => (row ? deviceActions(row.host) : []), [row]);
  const primary = useMemo(() => (row ? primaryAction(row.host) : null), [row]);
  const deviceId = row?.device_id ?? null;

  if (!open || !row) return null;

  // Only trust a detail that belongs to this row's device; it loads
  // asynchronously and can briefly describe the previous selection.
  const rowDetail = detail && detail.device.id === row.device_id ? detail : null;
  const status = rowDetail?.device.status ?? row.status;
  // Presence is a fact about the latest completed scan, so it is only ever the
  // backend's answer; without one the honest value is Unknown.
  const presence = rowDetail?.presence ?? "unknown";
  const storedNotes = notesFromDetail(row, detail);

  return (
    <Drawer
      open={open}
      onClose={onClose}
      overlay={overlay}
      width={width}
      onWidthChange={onWidthChange}
      title={rowName(row)}
      subtitle={
        <span className="mono">
          {row.host.ip}
          {row.host.mac ? ` · ${row.host.mac}` : ""}
        </span>
      }
      footer={
        <div className="flex flex-wrap items-center gap-1.5">
          {primary ? (
            <Button
              variant="primary"
              size="sm"
              icon={ACTION_ICON[primary.id]}
              onClick={() => onAction(primary.id, row, primary.port)}
              title={primary.hint}
            >
              {primary.label}
            </Button>
          ) : null}
          {actions
            .filter((action) => action.id !== primary?.id)
            .map((action) => (
              <Button
                key={action.id}
                size="sm"
                variant={action.emphasised ? "secondary" : "ghost"}
                disabled={!action.available}
                icon={ACTION_ICON[action.id]}
                title={action.hint}
                onClick={() => onAction(action.id, row, action.port)}
              >
                {SHORT_LABEL[action.id]}
              </Button>
            ))}
        </div>
      }
    >
      <div className="space-y-4">
        <section>
          <SectionHeading>Identity</SectionHeading>
          <label className="field-label" htmlFor="device-name">
            Name
          </label>
          <input
            id="device-name"
            className="field"
            value={draft.name}
            placeholder={row.host.hostname ?? "Give this device a name"}
            disabled={deviceId == null}
            onChange={(event) =>
              setDraft((current) => ({ ...current, name: event.target.value, nameDirty: true }))
            }
            onBlur={() => {
              if (deviceId == null) return;
              const next = draft.name.trim();
              if (next !== (row.custom_name ?? "")) onRename(deviceId, next || null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
            }}
          />
          <p className="mt-1 text-xs text-text-muted">
            {deviceId == null
              ? "Naming becomes available once the scan has been saved."
              : "Your name is used everywhere instead of the hostname."}
          </p>
        </section>

        <div className="divider" />

        <section>
          <SectionHeading>Status</SectionHeading>
          <Select
            aria-label="Device status"
            value={status}
            disabled={deviceId == null}
            onChange={(event) => {
              if (deviceId != null) onStatusChange(deviceId, event.target.value as DeviceStatus);
            }}
          >
            {(Object.keys(STATUS_LABEL) as DeviceStatus[]).map((value) => (
              <option key={value} value={value}>
                {STATUS_LABEL[value]}
              </option>
            ))}
          </Select>
          <p className="mt-1 text-xs leading-relaxed text-text-muted">{STATUS_HINT[status]}</p>
        </section>

        <div className="divider" />

        <section>
          <SectionHeading>Details</SectionHeading>
          <dl>
            <DetailRow label="State">
              <span className="inline-flex flex-wrap items-center gap-1.5">
                {context === "scan" ? (
                  // In a scan the device demonstrably answered, which is a
                  // different and stronger claim than a presence verdict.
                  <Badge tone="online">Answered this scan</Badge>
                ) : (
                  <Badge
                    tone={
                      presence === "present"
                        ? "online"
                        : presence === "missing"
                          ? "missing"
                          : "neutral"
                    }
                    title={PRESENCE_HINT[presence]}
                  >
                    {presence === "present"
                      ? "Present in latest scan"
                      : presence === "missing"
                        ? "Missing from latest scan"
                        : "Presence unknown"}
                  </Badge>
                )}
                {row.change === "new" ? <Badge tone="new">New device</Badge> : null}
                {row.change === "returned" ? <Badge tone="accent">Returned</Badge> : null}
                {row.change === "changed" ? <Badge tone="changed">Changed</Badge> : null}
              </span>
            </DetailRow>
            {rowDetail?.network_name ? (
              <DetailRow label="Network">{rowDetail.network_name}</DetailRow>
            ) : null}
            <DetailRow label="IP address" mono>
              {row.host.ip}
            </DetailRow>
            <DetailRow label="MAC address" mono>
              {row.host.mac ?? (
                <span className="text-text-muted">
                  Not available. MAC addresses are only visible on your local segment.
                </span>
              )}
            </DetailRow>
            <DetailRow label="Manufacturer">
              {row.host.vendor ?? <span className="text-text-muted">Unknown</span>}
            </DetailRow>
            <DetailRow label="Hostname">
              {row.host.hostname ?? <span className="text-text-muted">No reverse DNS record</span>}
            </DetailRow>
            <DetailRow label="OS guess">
              {row.host.os_guess ?? <span className="text-text-muted">Unknown</span>}
            </DetailRow>
            <DetailRow label="TTL" mono>
              {/* An inventory row carries no TTL of its own, so the value comes
                  from the most recent stored observation rather than reading as
                  "no reply" for a device that answered perfectly well. */}
              {row.host.ttl ?? rowDetail?.observations[0]?.ttl ?? (
                <span className="font-sans text-text-muted">No ICMP reply</span>
              )}
            </DetailRow>
            <DetailRow label="ICMP" mono>
              {formatLatency(row.host.icmp_ms) ?? (
                <span className="font-sans text-text-muted">No reply</span>
              )}
            </DetailRow>
            <DetailRow label="TCP connect" mono>
              {formatLatency(row.host.tcp_ms) ?? (
                <span className="font-sans text-text-muted">No reply</span>
              )}
            </DetailRow>
            {rowDetail ? (
              <>
                <DetailRow label="First seen">
                  <span title={rowDetail.device.first_seen}>
                    {formatDateTime(rowDetail.device.first_seen)}
                  </span>
                </DetailRow>
                <DetailRow label="Last seen">
                  <span title={rowDetail.device.last_seen}>
                    {formatDateTime(rowDetail.device.last_seen)}
                  </span>
                </DetailRow>
                <DetailRow label="Observations">
                  {rowDetail.device.observation_count === 1
                    ? "1 scan"
                    : `${rowDetail.device.observation_count} scans`}
                </DetailRow>
              </>
            ) : null}
          </dl>
        </section>

        <div className="divider" />

        <section>
          <SectionHeading>Open services</SectionHeading>
          {row.host.open_ports.length === 0 ? (
            <p className="text-[13px] text-text-secondary">
              No probed port accepted a connection. The device answered ICMP or ARP, so it is on the
              network with nothing listening on the ports this profile checked.
            </p>
          ) : (
            <ul className="space-y-1">
              {row.host.open_ports.map((port) => (
                <li key={port} className="flex items-center justify-between gap-3">
                  <span className="mono text-[13px] text-text">{serviceWithPort(port)}</span>
                  {isSensitivePort(port) ? (
                    <Badge tone="warning">Remote access</Badge>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </section>

        {rowDetail && rowDetail.previous_ips.length > 1 ? (
          <>
            <div className="divider" />
            <section>
              <SectionHeading>Previous addresses</SectionHeading>
              <ul className="mono space-y-0.5 text-[13px] text-text-secondary">
                {rowDetail.previous_ips.slice(1).map((ip) => (
                  <li key={ip}>{ip}</li>
                ))}
              </ul>
            </section>
          </>
        ) : null}

        {rowDetail && rowDetail.events.length > 0 ? (
          <>
            <div className="divider" />
            <section>
              <SectionHeading>Recorded changes</SectionHeading>
              <EventList events={rowDetail.events} highlightId={highlightEventId} />
            </section>
          </>
        ) : rowDetail && rowDetail.recent_changes.length > 0 ? (
          <>
            <div className="divider" />
            <section>
              <SectionHeading>Since the previous sighting</SectionHeading>
              <ChangeList changes={rowDetail.recent_changes} />
            </section>
          </>
        ) : null}

        <div className="divider" />

        <section>
          <SectionHeading>Notes</SectionHeading>
          <textarea
            className="field field-textarea min-h-[4.5rem]"
            value={draft.notes}
            placeholder="Anything worth remembering about this device"
            disabled={deviceId == null}
            onChange={(event) =>
              setDraft((current) => ({ ...current, notes: event.target.value, notesDirty: true }))
            }
            onBlur={() => {
              if (deviceId == null) return;
              const next = draft.notes.trim();
              if (next !== (storedNotes ?? "")) onNotesChange(deviceId, next || null);
            }}
          />
        </section>

        {rowDetail && rowDetail.observations.length > 0 ? (
          <>
            <div className="divider" />
            <section>
              <SectionHeading>Scan history</SectionHeading>
              <ul className="space-y-1.5">
                {rowDetail.observations.slice(0, 12).map((observation) => (
                  <li
                    key={`${observation.scan_id}-${observation.ip}`}
                    className="flex items-baseline justify-between gap-3 text-[13px]"
                  >
                    <span className="mono text-text-secondary">{observation.ip}</span>
                    <span className="text-xs text-text-muted" title={observation.observed_at}>
                      {formatRelative(observation.observed_at)}
                    </span>
                  </li>
                ))}
              </ul>
              {rowDetail.observations.length > 12 ? (
                <p className="mt-1.5 text-xs text-text-muted">
                  Showing the 12 most recent of {rowDetail.observations.length} sightings.
                </p>
              ) : null}
            </section>
          </>
        ) : loading ? (
          <p className="text-xs text-text-muted">Loading this device's history…</p>
        ) : null}
      </div>
    </Drawer>
  );
}

export function ChangeList({ changes }: { changes: FieldChange[] }) {
  return (
    <ul className="space-y-2">
      {changes.map((change) => (
        <li key={change.field}>
          <p className="text-xs font-semibold text-text-secondary">{change.label}</p>
          {change.field === "ports" ? (
            <div className="mt-0.5 space-y-0.5">
              {change.added_ports.map((port) => (
                <p key={`add-${port}`} className="mono text-[13px] text-online">
                  + {serviceWithPort(port)}
                </p>
              ))}
              {change.removed_ports.map((port) => (
                <p key={`rem-${port}`} className="mono text-[13px] text-missing">
                  − {serviceWithPort(port)}
                </p>
              ))}
            </div>
          ) : (
            <p className="mono mt-0.5 text-[13px] text-text">
              <span className="text-text-muted line-through">{change.from ?? "none"}</span>
              <span className="px-1.5 text-text-muted" aria-label="changed to">
                →
              </span>
              <span>{change.to ?? "none"}</span>
            </p>
          )}
        </li>
      ))}
    </ul>
  );
}

const ACTION_ICON: Record<ActionId, React.ReactNode> = {
  copy: <Copy className="h-3.5 w-3.5" />,
  web: <Globe className="h-3.5 w-3.5" />,
  smb: <FolderOpen className="h-3.5 w-3.5" />,
  rdp: <Monitor className="h-3.5 w-3.5" />,
  ssh: <TerminalSquare className="h-3.5 w-3.5" />,
  wol: <Power className="h-3.5 w-3.5" />,
};

const SHORT_LABEL: Record<ActionId, string> = {
  copy: "Copy IP",
  web: "Web",
  smb: "Shares",
  rdp: "RDP",
  ssh: "SSH",
  wol: "Wake",
};

/** Icons for the status column, exported so the table and drawer agree. */
export const STATUS_ICON: Record<DeviceStatus, React.ReactNode> = {
  unclassified: null,
  known: <Star className="h-3.5 w-3.5" />,
  trusted: <ShieldCheck className="h-3.5 w-3.5" />,
  watched: <Eye className="h-3.5 w-3.5" />,
  ignored: <EyeOff className="h-3.5 w-3.5" />,
};

/**
 * Persisted change events for one device.
 *
 * Old and new values are read out in full rather than shown as a bare arrow, so
 * a screen reader hears "IP address, from 192.168.1.28 to 192.168.1.31" instead
 * of two numbers with a symbol between them.
 */
function EventList({
  events,
  highlightId,
}: {
  events: ChangeEvent[];
  highlightId: number | null;
}) {
  return (
    <ul className="space-y-2">
      {events.slice(0, 12).map((event) => (
        <li
          key={event.id}
          className={
            event.id === highlightId
              ? "rounded-md border border-accent/50 bg-accent-subtle px-2 py-1.5"
              : undefined
          }
        >
          <p className="flex flex-wrap items-baseline gap-x-2 text-xs font-semibold text-text-secondary">
            {CHANGE_TYPE_LABEL[event.change_type]}
            <span className="font-normal text-text-muted" title={event.scan_at ?? event.created_at}>
              {formatRelative(event.scan_at ?? event.created_at)}
            </span>
            {event.state === "acknowledged" ? <Badge>Acknowledged</Badge> : null}
            {event.state === "ignored" ? <Badge>Ignored</Badge> : null}
          </p>
          {event.change_type === "ports_changed" ? (
            <div className="mt-0.5 space-y-0.5">
              {event.opened_ports.map((port) => (
                <p key={`add-${port}`} className="mono text-[13px] text-online">
                  Opened {serviceWithPort(port)}
                </p>
              ))}
              {event.closed_ports.map((port) => (
                <p key={`rem-${port}`} className="mono text-[13px] text-missing">
                  Closed {serviceWithPort(port)}
                </p>
              ))}
            </div>
          ) : (
            <p className="mono mt-0.5 text-[13px] text-text">{describeChange(event)}</p>
          )}
        </li>
      ))}
    </ul>
  );
}
