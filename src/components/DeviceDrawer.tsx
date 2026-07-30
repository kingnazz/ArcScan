// The device detail panel.
//
// Typography and dividers rather than a card per section, so the panel reads as
// one document. Actions are always in the same place and in the same order, but
// only the ones the open services support are enabled, and the one the operator
// most likely wants is emphasised.

import { useEffect, useMemo, useRef, useState } from "react";
import {
  Copy,
  Eye,
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
import { rowName, type DeviceRow } from "../lib/live";
import type { DeviceDetail, DeviceStatus, FieldChange } from "../types";

const STATUS_LABEL: Record<DeviceStatus, string> = {
  unclassified: "Not classified",
  known: "Known",
  trusted: "Trusted",
  watched: "Watched",
};

const STATUS_HINT: Record<DeviceStatus, string> = {
  unclassified: "Seen on the network, never labelled.",
  known: "Recognised and expected here.",
  trusted: "Recognised, and its open services are deliberate.",
  watched: "Recognised, and worth a look whenever it changes.",
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
}: DeviceDrawerProps) {
  const [nameDraft, setNameDraft] = useState("");
  const [notesDraft, setNotesDraft] = useState("");
  const editingIp = useRef<string | null>(null);

  // Reset the drafts when the panel moves to a different device, but not on every
  // re-render, or typing would be wiped by an incoming scan event.
  useEffect(() => {
    if (!row) return;
    if (editingIp.current === row.host.ip) return;
    editingIp.current = row.host.ip;
    setNameDraft(row.custom_name ?? "");
    setNotesDraft(detail?.device.notes ?? "");
  }, [row, detail]);

  useEffect(() => {
    if (detail && editingIp.current === detail.device.last_ip) {
      setNotesDraft((current) => current || (detail.device.notes ?? ""));
    }
  }, [detail]);

  const actions = useMemo(() => (row ? deviceActions(row.host) : []), [row]);
  const primary = useMemo(() => (row ? primaryAction(row.host) : null), [row]);
  const deviceId = row?.device_id ?? detail?.device.id ?? null;

  if (!open || !row) return null;

  const status = detail?.device.status ?? row.status;

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
            value={nameDraft}
            placeholder={row.host.hostname ?? "Give this device a name"}
            disabled={deviceId == null}
            onChange={(event) => setNameDraft(event.target.value)}
            onBlur={() => {
              if (deviceId == null) return;
              const next = nameDraft.trim();
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
              <span className="inline-flex items-center gap-1.5">
                <Badge tone="online">Online</Badge>
                {row.change === "new" ? <Badge tone="new">New device</Badge> : null}
                {row.change === "returned" ? <Badge tone="accent">Returned</Badge> : null}
                {row.change === "changed" ? <Badge tone="changed">Changed</Badge> : null}
              </span>
            </DetailRow>
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
              {row.host.ttl ?? <span className="font-sans text-text-muted">No ICMP reply</span>}
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
            {detail ? (
              <>
                <DetailRow label="First seen">
                  <span title={detail.device.first_seen}>
                    {formatDateTime(detail.device.first_seen)}
                  </span>
                </DetailRow>
                <DetailRow label="Observations">
                  {detail.device.observation_count === 1
                    ? "1 scan"
                    : `${detail.device.observation_count} scans`}
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

        {detail && detail.previous_ips.length > 1 ? (
          <>
            <div className="divider" />
            <section>
              <SectionHeading>Previous addresses</SectionHeading>
              <ul className="mono space-y-0.5 text-[13px] text-text-secondary">
                {detail.previous_ips.slice(1).map((ip) => (
                  <li key={ip}>{ip}</li>
                ))}
              </ul>
            </section>
          </>
        ) : null}

        {detail && detail.recent_changes.length > 0 ? (
          <>
            <div className="divider" />
            <section>
              <SectionHeading>Recent changes</SectionHeading>
              <ChangeList changes={detail.recent_changes} />
            </section>
          </>
        ) : null}

        <div className="divider" />

        <section>
          <SectionHeading>Notes</SectionHeading>
          <textarea
            className="field field-textarea min-h-[4.5rem]"
            value={notesDraft}
            placeholder="Anything worth remembering about this device"
            disabled={deviceId == null}
            onChange={(event) => setNotesDraft(event.target.value)}
            onBlur={() => {
              if (deviceId == null) return;
              const next = notesDraft.trim();
              if (next !== (detail?.device.notes ?? "")) onNotesChange(deviceId, next || null);
            }}
          />
        </section>

        {detail && detail.observations.length > 0 ? (
          <>
            <div className="divider" />
            <section>
              <SectionHeading>Scan history</SectionHeading>
              <ul className="space-y-1.5">
                {detail.observations.slice(0, 12).map((observation) => (
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
              {detail.observations.length > 12 ? (
                <p className="mt-1.5 text-xs text-text-muted">
                  Showing the 12 most recent of {detail.observations.length} sightings.
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
};
