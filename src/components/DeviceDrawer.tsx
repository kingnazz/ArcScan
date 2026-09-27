// The device inspector.
//
// Read top to bottom, it answers the questions in the order a technician asks
// them: what is this, where is it, what is it running, what have I said about
// it, how does ArcScan know, and what has changed. Facts are a two-column grid
// rather than a form, the actions sit under the name where the eye already is,
// and the evidence behind the answers is one click away instead of a second
// screen of rows at the same weight as the device's address.
//
// Actions are always in the same place and in the same order, but only the ones
// the open services support are enabled, and the one the operator most likely
// wants is emphasised.

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
import { Badge, Button, IconButton, Select } from "../ui/primitives";
import { Drawer } from "../ui/Drawer";
import { Disclosure, Fact, Section } from "../ui/patterns";
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
import {
  DEVICE_TYPE_LABEL,
  WINDOWS_PRODUCT_TYPE_HINT,
  deviceTypeLabel,
  evidenceKindLabel,
  osSummary,
  serviceName,
  sourceLabel,
  sourcesLabel,
} from "../lib/discovery";
import {
  FRESHNESS_HINT,
  detectedUnderOverride,
  effectiveTypeSummary,
  freshnessLabel,
  isNoteworthyFreshness,
  missPhrase,
  resolveType,
  type EffectiveType,
} from "../lib/effectiveType";
import type {
  ChangeEvent,
  DeviceDetail,
  DeviceDiscovery,
  DeviceStatus,
  DiscoveryEvidenceRow,
  FieldChange,
} from "../types";

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
  /**
   * Correct, change or clear the detected device type.
   *
   * Resolves `false` when the save failed, which is what lets the editor put
   * the previous value back rather than leave the drawer showing a type the
   * database does not hold.
   */
  onTypeChange?: (deviceId: number, deviceType: string | null) => Promise<boolean>;
  /** Copy the redacted discovery report for this device. */
  onCopyDiscovery?: (deviceId: number) => void;
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
  onTypeChange,
  onCopyDiscovery,
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
  const discovery = rowDetail?.discovery ?? null;

  const meta = (
    <>
      {context === "scan" ? (
        // In a scan the device demonstrably answered, which is a different and
        // stronger claim than a presence verdict.
        <Badge tone="online">Answered this scan</Badge>
      ) : (
        <Badge
          tone={presence === "present" ? "online" : presence === "missing" ? "missing" : "neutral"}
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
      {status !== "unclassified" ? (
        <Badge icon={STATUS_ICON[status]} title={STATUS_HINT[status]}>
          {STATUS_LABEL[status]}
        </Badge>
      ) : null}
    </>
  );

  // Available actions keep their labels; unavailable ones gather at the end as
  // icons with the reason on hover. Each group keeps the same order on every
  // device, so muscle memory holds, and nothing that does nothing competes with
  // what does.
  const others = actions.filter((action) => action.id !== primary?.id);
  const unavailable = others.filter((action) => !action.available);
  const toolbar = (
    <>
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
      {others
        .filter((action) => action.available)
        .map((action) => (
          <Button
            key={action.id}
            size="sm"
            variant="ghost"
            icon={ACTION_ICON[action.id]}
            title={action.hint}
            onClick={() => onAction(action.id, row, action.port)}
          >
            {SHORT_LABEL[action.id]}
          </Button>
        ))}
      {unavailable.length > 0 ? (
        <span className="ml-auto flex items-center" aria-label="Unavailable for this device" role="group">
          {unavailable.map((action) => (
            <IconButton
              key={action.id}
              size="sm"
              label={`${SHORT_LABEL[action.id]}: ${action.hint}`}
              disabled
            >
              {ACTION_ICON[action.id]}
            </IconButton>
          ))}
        </span>
      ) : null}
    </>
  );

  const events = rowDetail?.events ?? [];
  const recentChanges = rowDetail?.recent_changes ?? [];

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
      meta={meta}
      toolbar={toolbar}
      flush
    >
      {/* Opened from Changes: what changed comes first, because it is why the
          operator is here. */}
      {highlightEventId != null && events.length > 0 ? (
        <Section title="Recorded changes">
          <EventList events={events} highlightId={highlightEventId} />
        </Section>
      ) : null}

      <IdentitySection
        row={row}
        discovery={discovery}
        detailLoaded={rowDetail != null}
        deviceId={deviceId}
        userType={rowDetail?.device.user_device_type ?? null}
        onTypeChange={onTypeChange}
      />

      <Section title="Network">
        <dl className="facts">
          {rowDetail?.network_name ? <Fact label="Network">{rowDetail.network_name}</Fact> : null}
          <Fact label="IP address" mono>
            {row.host.ip}
          </Fact>
          <Fact label="MAC address" mono={row.host.mac != null}>
            {row.host.mac ?? (
              <span className="text-text-muted">Only visible on this computer's own network</span>
            )}
          </Fact>
          <Fact label="Hostname">
            {row.host.hostname ?? <span className="text-text-muted">No reverse DNS record</span>}
          </Fact>
          {discovery && discovery.ipv6_addresses.length > 0 ? (
            <Fact label="IPv6" mono>
              <span className="break-words">{discovery.ipv6_addresses.join(", ")}</span>
              {/* Said plainly, because showing a v6 address next to a scanner
                  that only speaks v4 would otherwise imply it was scanned. */}
              <span className="block font-sans text-xs text-text-muted">
                Learned from mDNS. ArcScan scans IPv4 only.
              </span>
            </Fact>
          ) : null}
          <Fact label="Response">
            <span className="mono">
              {formatLatency(row.host.icmp_ms) ?? "—"}
              <span className="font-sans text-text-muted"> ICMP</span>
              <span className="text-text-muted"> · </span>
              {formatLatency(row.host.tcp_ms) ?? "—"}
              <span className="font-sans text-text-muted"> TCP</span>
            </span>
          </Fact>
          <Fact label="TTL" mono>
            {/* An inventory row carries no TTL of its own, so the value comes
                from the most recent stored observation rather than reading as
                "no reply" for a device that answered perfectly well. */}
            {row.host.ttl ?? rowDetail?.observations[0]?.ttl ?? (
              <span className="font-sans text-text-muted">No ICMP reply</span>
            )}
          </Fact>
          {rowDetail ? (
            <>
              <Fact label="First seen" title={rowDetail.device.first_seen}>
                {formatDateTime(rowDetail.device.first_seen)}
              </Fact>
              <Fact label="Last seen" title={rowDetail.device.last_seen}>
                {formatDateTime(rowDetail.device.last_seen)}
              </Fact>
              <Fact label="Seen in">
                {rowDetail.device.observation_count === 1
                  ? "1 scan"
                  : `${rowDetail.device.observation_count} scans`}
              </Fact>
            </>
          ) : null}
        </dl>
      </Section>

      <Section title="Services">
        {row.host.open_ports.length === 0 ? (
          <p className="text-[13px] leading-relaxed text-text-secondary">
            No probed port accepted a connection. The device answered ICMP or ARP, so it is on the
            network with nothing listening on the ports this profile checked.
          </p>
        ) : (
          <ul className="space-y-1">
            {row.host.open_ports.map((port) => (
              <li key={port} className="flex items-center justify-between gap-3">
                <span className="mono text-[13px] text-text">{serviceWithPort(port)}</span>
                {isSensitivePort(port) ? <Badge tone="warning">Remote access</Badge> : null}
              </li>
            ))}
          </ul>
        )}
        {discovery && discovery.services.length > 0 ? (
          <div className="mt-3">
            <p className="text-xs text-text-muted">Advertised on the local network</p>
            <ul className="mt-1 space-y-0.5">
              {discovery.services.map((service) => (
                <li key={service} className="text-[13px] text-text-secondary">
                  <span className="break-words">{serviceName(service)}</span>
                  <span className="mono ml-1.5 text-xs text-text-muted">{service}</span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </Section>

      <Section title="Your labels">
        <div className="space-y-3">
          <div>
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
          </div>

          <div>
            <label className="field-label" htmlFor="device-status">
              Status
            </label>
            <Select
              id="device-status"
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
          </div>

          <div>
            <label className="field-label" htmlFor="device-notes">
              Notes
            </label>
            <textarea
              id="device-notes"
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
          </div>
        </div>
      </Section>

      {discovery || deviceId != null ? (
        <EvidenceSection
          discovery={discovery}
          deviceId={deviceId}
          onCopyDiscovery={onCopyDiscovery}
        />
      ) : null}

      {highlightEventId == null && events.length > 0 ? (
        <Section title="Recorded changes">
          <EventList events={events} highlightId={null} />
        </Section>
      ) : events.length === 0 && recentChanges.length > 0 ? (
        <Section title="Since the previous sighting">
          <ChangeList changes={recentChanges} />
        </Section>
      ) : null}

      {rowDetail && (rowDetail.observations.length > 0 || rowDetail.previous_ips.length > 1) ? (
        <Section title="History">
          <div className="space-y-2">
            {rowDetail.previous_ips.length > 1 ? (
              <div>
                <p className="text-xs text-text-muted">Previous addresses</p>
                <ul className="mono mt-1 space-y-0.5 text-[13px] text-text-secondary">
                  {rowDetail.previous_ips.slice(1).map((ip) => (
                    <li key={ip}>{ip}</li>
                  ))}
                </ul>
              </div>
            ) : null}
            {rowDetail.observations.length > 0 ? (
              <Disclosure summary="Sightings" count={rowDetail.observations.length}>
                <ul className="space-y-1">
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
              </Disclosure>
            ) : null}
          </div>
        </Section>
      ) : loading ? (
        <p className="px-4 py-3 text-xs text-text-muted">Loading this device's history…</p>
      ) : null}
    </Drawer>
  );
}

/**
 * What the device is: its type, the name and model it gave, what it runs.
 *
 * Every value here came off the network from an unauthenticated device, so it
 * is rendered as text — React escapes it, the Rust parsers already bounded its
 * length and stripped its control characters, and `break-words` keeps a device
 * that advertises a 128-character model number from breaking the layout.
 *
 * It never competes with the name at the top of the inspector: when the
 * operator has named the device, the detected name is shown as something
 * ArcScan found, not as a correction.
 */
function IdentitySection({
  row,
  discovery,
  detailLoaded,
  deviceId,
  userType,
  onTypeChange,
}: {
  row: DeviceRow;
  discovery: DeviceDiscovery | null;
  detailLoaded: boolean;
  deviceId: number | null;
  userType: string | null;
  onTypeChange?: (deviceId: number, deviceType: string | null) => Promise<boolean>;
}) {
  const resolved = resolveType({
    userOverride: userType,
    detectedType: discovery?.device_type,
    detectedConfidence: discovery?.type_confidence,
  });
  const detectedLine = detectedUnderOverride(resolved);
  const stale = discovery?.evidence_freshness === "stale";
  // A reduction is only worth explaining when there is one to explain.
  const reduced =
    stale &&
    discovery != null &&
    discovery.raw_type_confidence === "high" &&
    discovery.type_confidence !== "high";
  // One line, or null when nothing established a product. Null renders nothing
  // rather than an empty row: a column of dashes reads as "asked and found
  // nothing", which is not what a Quick Scan did.
  const osLine = osSummary(discovery);
  const hasUserName = Boolean(row.custom_name?.trim());

  return (
    <Section title="Identity">
      <TypeEditor deviceId={deviceId} userType={userType} resolved={resolved} onTypeChange={onTypeChange} />
      {detectedLine ? (
        <p className="mt-1 text-xs text-text-muted">
          {/* Kept underneath rather than replaced, so clearing the correction
              is never a loss and the operator can see what they overruled. */}
          ArcScan detected: <span className="text-text-secondary">{detectedLine}</span>
        </p>
      ) : null}
      {reduced ? (
        <p className="mt-1 text-xs text-text-muted">
          Reduced from high confidence: nothing has confirmed this in the last few scans that could
          have.
        </p>
      ) : null}
      {/* A device that looks like two things is shown as such rather than
          having the disagreement resolved silently. */}
      {discovery && discovery.type_conflicts.length > 0 ? (
        <p className="mt-1 text-xs text-text-muted">
          Also consistent with:{" "}
          <span className="text-text-secondary">{discovery.type_conflicts.join(" · ")}</span>
        </p>
      ) : null}

      <dl className="facts mt-3">
        <Fact label="Manufacturer">
          {discovery?.manufacturer ?? row.host.vendor ?? <span className="text-text-muted">Unknown</span>}
        </Fact>
        {discovery?.detected_name ? (
          <Fact label="Detected as">
            <span className="break-words">{discovery.detected_name}</span>
            {hasUserName ? (
              <span className="mt-0.5 block text-xs text-text-muted">
                Your name is used everywhere instead.
              </span>
            ) : null}
          </Fact>
        ) : null}
        {discovery?.model_name ? (
          <Fact label="Model">
            <span className="break-words">
              {discovery.model_name}
              {discovery.model_number && discovery.model_number !== discovery.model_name
                ? ` (${discovery.model_number})`
                : ""}
            </span>
          </Fact>
        ) : null}
        {/* The operating system and hardware a deep or credentialed scan
            established, else the TTL-based guess a Quick scan makes. */}
        {osLine ? (
          <Fact label="Operating system">
            <span className="break-words">{osLine}</span>
            {discovery?.windows_product_type ? (
              <span className="mt-0.5 block text-xs text-text-muted">
                {WINDOWS_PRODUCT_TYPE_HINT[discovery.windows_product_type] ??
                  `Windows ProductType ${discovery.windows_product_type}.`}
              </span>
            ) : null}
          </Fact>
        ) : (
          <Fact label="OS guess" title={row.host.ttl != null ? `Guessed from TTL ${row.host.ttl}` : undefined}>
            {row.host.os_guess ?? <span className="text-text-muted">Unknown</span>}
          </Fact>
        )}
        {discovery?.hardware_model ? (
          <Fact label="Hardware">
            <span className="break-words">
              {[discovery.hardware_manufacturer, discovery.hardware_model].filter(Boolean).join(" ")}
            </span>
          </Fact>
        ) : null}
        {discovery?.hardware_serial ? (
          <Fact label="Serial / service tag" mono>
            <span className="break-words">{discovery.hardware_serial}</span>
          </Fact>
        ) : null}
        {discovery?.system_uuid ? (
          <Fact label="System UUID" mono>
            <span className="break-words">{discovery.system_uuid}</span>
          </Fact>
        ) : null}
        {discovery?.domain ? (
          <Fact label="Domain">
            <span className="break-words">{discovery.domain}</span>
          </Fact>
        ) : null}
        {discovery?.mdns_hostname ? (
          <Fact label="mDNS host name" mono>
            <span className="break-words">{discovery.mdns_hostname}</span>
          </Fact>
        ) : null}
        {discovery?.ssdp_friendly_name && discovery.ssdp_friendly_name !== discovery.detected_name ? (
          <Fact label="SSDP name">
            <span className="break-words">{discovery.ssdp_friendly_name}</span>
          </Fact>
        ) : null}
      </dl>

      {discovery == null && detailLoaded ? (
        <p className="mt-3 text-xs leading-relaxed text-text-muted">
          No discovery-capable scan has reached this device, so ArcScan has nothing of its own to go
          on. You can still set the type yourself.
        </p>
      ) : null}
    </Section>
  );
}

/**
 * How ArcScan knows: which protocols heard the device, how fresh that is, and
 * the evidence itself.
 *
 * The summary stays visible; the evidence rows are behind a disclosure because
 * they matter when a technician doubts an answer, not on every glance. Evidence
 * ArcScan has stopped hearing is kept and dated rather than deleted, counted in
 * scans and never in days: ArcScan only learns when it runs.
 */
function EvidenceSection({
  discovery,
  deviceId,
  onCopyDiscovery,
}: {
  discovery: DeviceDiscovery | null;
  deviceId: number | null;
  onCopyDiscovery?: (deviceId: number) => void;
}) {
  const evidence = discovery?.evidence ?? [];
  const staleEvidence = evidence.filter((row) => row.freshness === "stale");
  const agingEvidence = evidence.filter((row) => row.freshness === "aging");
  const stale = discovery?.evidence_freshness === "stale";
  const detailCount =
    (discovery?.type_evidence.length ?? 0) +
    (discovery?.identity_evidence?.length ?? 0) +
    agingEvidence.length +
    staleEvidence.length +
    (discovery?.alternate_names.length ?? 0);

  return (
    <Section
      title="Discovery"
      action={
        deviceId != null && onCopyDiscovery ? (
          <Button
            size="sm"
            variant="ghost"
            icon={<Copy className="h-3.5 w-3.5" />}
            onClick={() => onCopyDiscovery(deviceId)}
            title="A redacted summary for reporting bad detection. It leaves out your notes, the MAC address, the serial number and the full IP address, and nothing is sent anywhere."
          >
            Copy discovery details
          </Button>
        ) : undefined
      }
    >
      {discovery ? (
        <>
          <dl className="facts">
            <Fact label="Discovered by">{sourcesLabel(discovery.sources)}</Fact>
            <Fact label="Evidence">
              <Badge
                tone={stale ? "warning" : discovery.evidence_freshness === "aging" ? "accent" : "online"}
                title={FRESHNESS_HINT[discovery.evidence_freshness] ?? FRESHNESS_HINT.current}
              >
                {freshnessLabel(discovery.evidence_freshness)}
              </Badge>
            </Fact>
            {discovery.last_discovered_at ? (
              <Fact label="Last discovered" title={discovery.last_discovered_at}>
                {formatRelative(discovery.last_discovered_at)}
              </Fact>
            ) : null}
          </dl>

          {detailCount > 0 ? (
            <div className="mt-2.5">
              <Disclosure summary="Evidence" count={detailCount}>
                <div className="space-y-2.5">
                  {discovery.type_evidence.length > 0 ? (
                    <EvidenceList heading="Why this type">
                      {discovery.type_evidence.map((line) => (
                        <li key={line} className="break-words text-[13px] text-text-secondary">
                          {line}
                        </li>
                      ))}
                    </EvidenceList>
                  ) : null}
                  {discovery.identity_evidence && discovery.identity_evidence.length > 0 ? (
                    // What ArcScan would use to recognise this device again, and
                    // to tell it apart from a second machine with the same name.
                    // Strongest first.
                    <EvidenceList heading="How it is recognised">
                      {discovery.identity_evidence.map((line) => (
                        <li key={line} className="break-words font-mono text-[12.5px] text-text-secondary">
                          {line}
                        </li>
                      ))}
                    </EvidenceList>
                  ) : null}
                  {agingEvidence.length > 0 ? (
                    <EvidenceGroup heading="Not heard recently" rows={agingEvidence} />
                  ) : null}
                  {staleEvidence.length > 0 ? (
                    <EvidenceGroup heading="Stale evidence" rows={staleEvidence} />
                  ) : null}
                  {discovery.alternate_names.length > 0 ? (
                    <EvidenceList heading="Other names it advertised">
                      {discovery.alternate_names.map((name) => (
                        <li key={name} className="break-words text-[13px] text-text-secondary">
                          {name}
                        </li>
                      ))}
                    </EvidenceList>
                  ) : null}
                </div>
              </Disclosure>
            </div>
          ) : null}

          {discovery.presentation_url ? (
            <p className="mt-2 text-xs text-text-muted">
              {/* Recorded, never opened: the Web action validates the address
                  itself, and a device-supplied URL is not a link to hand a
                  person. */}
              Advertises its own page at{" "}
              <span className="mono break-all text-text-secondary">{discovery.presentation_url}</span>.
            </p>
          ) : null}
        </>
      ) : (
        <p className="text-xs leading-relaxed text-text-muted">
          Nothing discovered yet. Local discovery runs with scans of this computer's own network.
        </p>
      )}
    </Section>
  );
}

function EvidenceList({ heading, children }: { heading: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-xs text-text-muted">{heading}</p>
      <ul className="mt-0.5 space-y-0.5">{children}</ul>
    </div>
  );
}

/**
 * The device type, and the control that corrects it.
 *
 * A plain `<select>` and a reset button, in the flow of the section rather than
 * behind a modal or a right-click: a correction is an ordinary edit and should
 * cost one keystroke to make and one to undo. The drawer stays open throughout.
 *
 * The value is optimistic and rolls back on failure, so a save the backend
 * refuses leaves the control showing what the database actually holds rather
 * than what the operator hoped it would.
 */
function TypeEditor({
  deviceId,
  userType,
  resolved,
  onTypeChange,
}: {
  deviceId: number | null;
  userType: string | null;
  resolved: EffectiveType;
  onTypeChange?: (deviceId: number, deviceType: string | null) => Promise<boolean>;
}) {
  // `""` is Auto. An explicit `"unknown"` is a different option and a different
  // answer: a person saying "ArcScan is wrong and I do not know either".
  const [pending, setPending] = useState<string | null>(null);
  const shown = pending ?? userType ?? "";
  useEffect(() => setPending(null), [deviceId, userType]);

  const editable = deviceId != null && onTypeChange != null;
  return (
    <>
      <label className="field-label" htmlFor="device-type">
        Device type
      </label>
      <Select
        id="device-type"
        aria-label="Device type"
        value={shown}
        disabled={!editable}
        onChange={(event) => {
          if (!editable) return;
          const next = event.target.value || null;
          const previous = userType;
          setPending(next ?? "");
          void onTypeChange(deviceId, next).then((ok) => {
            // Put the previous value back rather than leave the control showing
            // a type nothing stored.
            if (!ok) setPending(previous ?? "");
          });
        }}
      >
        <option value="">
          Automatic{resolved.detectedType !== "unknown"
            ? ` (${deviceTypeLabel(resolved.detectedType)})`
            : ""}
        </option>
        {Object.keys(DEVICE_TYPE_LABEL).map((id) => (
          <option key={id} value={id}>
            {DEVICE_TYPE_LABEL[id]}
          </option>
        ))}
      </Select>
      <p className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-text-muted">
        <span>
          {resolved.isUserSet ? "Set by you." : "Detected automatically."}{" "}
          {resolved.isUserSet ? "" : effectiveTypeSummary(resolved)}
        </span>
        {resolved.isUserSet && editable ? (
          <button
            type="button"
            className="rounded text-xs font-medium text-accent-text underline decoration-dotted underline-offset-2 hover:no-underline"
            onClick={() => {
              const previous = userType;
              setPending("");
              void onTypeChange(deviceId, null).then((ok) => {
                if (!ok) setPending(previous ?? "");
              });
            }}
          >
            Use automatic detection
          </button>
        ) : null}
      </p>
      {deviceId == null ? (
        <p className="mt-1 text-xs text-text-muted">
          Correcting the type becomes available once the scan has been saved.
        </p>
      ) : null}
    </>
  );
}

/**
 * A group of evidence ArcScan has stopped hearing.
 *
 * Each row says how many *discovery scans* have missed it, which is the only
 * count ArcScan actually observed. A wall-clock age would be a fact about when
 * the operator last ran a scan, dressed up as a fact about the device.
 */
function EvidenceGroup({
  heading,
  rows,
}: {
  heading: string;
  rows: DiscoveryEvidenceRow[];
}) {
  return (
    <div>
      <p className="text-xs text-text-muted">{heading}</p>
      <ul className="mt-0.5 space-y-0.5">
        {rows.slice(0, 8).map((row) => (
          <li
            key={`${row.source}-${row.kind}-${row.key}-${row.value}`}
            className="text-[13px] text-text-secondary"
          >
            <span className="break-words">
              {row.kind === "service" ? serviceName(row.value) : row.value}
            </span>
            <span className="ml-1.5 text-xs text-text-muted">
              {sourceLabel(row.source)} {evidenceKindLabel(row.kind).toLowerCase()}
              {isNoteworthyFreshness(row.freshness) && missPhrase(row.misses)
                ? ` · ${missPhrase(row.misses)}`
                : ""}
            </span>
          </li>
        ))}
      </ul>
      {rows.length > 8 ? (
        <p className="mt-1 text-xs text-text-muted">
          Showing 8 of {rows.length}. The rest are in Copy discovery details.
        </p>
      ) : null}
    </div>
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
