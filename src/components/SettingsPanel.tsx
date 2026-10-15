// Settings.
//
// Global preferences live here rather than on the scanning screen, so the scan
// surface stays about the scan. Six groups — Scanning, Discovery, Credentials,
// Networks, Appearance, Application — with a rail that jumps between them, and
// the privacy-relevant switches stated plainly rather than buried.

import { useEffect, useRef, useState } from "react";
import { Check, Copy, ExternalLink, FolderOpen, KeyRound, RotateCcw } from "lucide-react";
import { Button, Field, FieldRow, Select } from "../ui/primitives";
import { destinationLabel, type ArcAtlasConnection } from "../lib/arcatlas";
import { windowsCredentials } from "../lib/windowsCredential";
import type { WindowsCredentialStatus } from "../types";
import { Drawer } from "../ui/Drawer";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { PROFILES, PROFILE_ORDER } from "../lib/profiles";
import { COLUMNS, type SortKey } from "../lib/table";
import { OPTIONAL_INVENTORY_COLUMNS, type InventoryColumn } from "../lib/inventory";
import { formatCount, parsePorts } from "../lib/format";
import type { Settings } from "../lib/prefs";
import { editionLabel, type RuntimeInfo } from "../lib/runtime";
import type { PublicIpState } from "../hooks/usePublicIp";
import type { NetworkScope } from "../types";

export interface SettingsPanelProps {
  open: boolean;
  onClose: () => void;
  overlay: boolean;
  width: number;
  onWidthChange: (width: number) => void;
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onReset: () => void;
  version: string;
  native: boolean;
  /** Which edition this is, or null until the backend has answered. */
  runtime: RuntimeInfo | null;
  onOpenDataFolder: () => void;
  onOpenPortableDownloads: () => void;
  publicIp: PublicIpState;
  onClearPublicIp: () => void;
  onOpenPrivacy: () => void;
  /** Known network scopes; empty until a scan has been saved. */
  scopes: NetworkScope[];
  onRenameScope: (id: number, name: string) => void;
  /** The ArcAtlas connection, managed here rather than from a send button. */
  arcAtlas?: ArcAtlasConnection;
  onManageArcAtlas?: () => void;
}

/**
 * The six groups, in the order a technician reaches for them. Every group is
 * rendered — the rail jumps between them rather than hiding the rest — so the
 * whole panel stays searchable with the browser's find and reachable by Tab.
 */
const GROUPS = [
  { id: "settings-scanning", label: "Scanning" },
  { id: "settings-discovery", label: "Discovery" },
  { id: "settings-credentials", label: "Credentials" },
  { id: "settings-networks", label: "Networks" },
  { id: "settings-appearance", label: "Appearance" },
  { id: "settings-application", label: "Application" },
] as const;

/** Settings is wider than the device inspector: it holds forms, not facts. */
const SETTINGS_WIDTH = 600;

export function SettingsPanel(props: SettingsPanelProps) {
  const { settings, onChange } = props;
  const [confirmReset, setConfirmReset] = useState(false);
  const [current, setCurrent] = useState<string>(GROUPS[0].id);
  const bodyRef = useRef<HTMLDivElement>(null);
  const portCheck = parsePorts(settings.portSpec);
  const portable = props.runtime?.edition === "portable";

  // Mark the group being read in the rail as the panel scrolls.
  useEffect(() => {
    if (!props.open) return;
    const root = bodyRef.current?.closest(".overflow-y-auto") ?? null;
    if (!root || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((entry) => entry.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (visible[0]) setCurrent(visible[0].target.id);
      },
      { root, rootMargin: "0px 0px -70% 0px" },
    );
    for (const group of GROUPS) {
      const element = document.getElementById(group.id);
      if (element) observer.observe(element);
    }
    return () => observer.disconnect();
  }, [props.open, props.overlay]);

  const jump = (id: string) => {
    setCurrent(id);
    document.getElementById(id)?.scrollIntoView({ block: "start" });
  };

  return (
    <>
      <Drawer
        open={props.open}
        onClose={props.onClose}
        overlay={props.overlay}
        width={props.width}
        onWidthChange={props.onWidthChange}
        fixedWidth={SETTINGS_WIDTH}
        flush
        title="Settings"
        subtitle={`ArcScan ${version(props.version)}${props.native ? "" : " · browser preview"}`}
        footer={
          <Button
            size="sm"
            variant="ghost"
            icon={<RotateCcw className="h-3.5 w-3.5" />}
            onClick={() => setConfirmReset(true)}
          >
            Reset all settings
          </Button>
        }
      >
        <div ref={bodyRef} className="flex min-h-full">
          {/* The rail: too wide for the narrow overlay, where the groups'
              own headings are enough. */}
          {!props.overlay ? (
            <nav
              aria-label="Settings sections"
              className="sticky top-0 w-36 shrink-0 self-start border-r border-border px-2 py-3"
            >
              <ul className="space-y-0.5">
                {GROUPS.map((group) => (
                  <li key={group.id}>
                    <button
                      type="button"
                      onClick={() => jump(group.id)}
                      aria-current={current === group.id ? "location" : undefined}
                      className={`w-full rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors duration-fast ${
                        current === group.id
                          ? "bg-surface-active font-medium text-text"
                          : "text-text-secondary hover:bg-surface-hover hover:text-text"
                      }`}
                    >
                      {group.label}
                    </button>
                  </li>
                ))}
              </ul>
            </nav>
          ) : null}

          <div className="min-w-0 flex-1 px-5 pb-8">
            <Group id="settings-scanning" title="Scanning" lead="What a scan checks, and how hard it pushes.">
              <FieldRow
                label="Default profile"
                htmlFor="settings-profile"
                hint={PROFILES[settings.defaultProfile].detail}
              >
                <Select
                  id="settings-profile"
                  value={settings.defaultProfile}
                  onChange={(e) =>
                    onChange({ defaultProfile: e.target.value as Settings["defaultProfile"] })
                  }
                >
                  {PROFILE_ORDER.map((id) => (
                    <option key={id} value={id}>
                      {PROFILES[id].name}
                    </option>
                  ))}
                </Select>
              </FieldRow>

              <FieldRow
                label="Ports for Custom and Full TCP"
                htmlFor="settings-ports"
                error={portCheck.error}
                hint={
                  portCheck.ports.length > 0
                    ? `${formatCount(portCheck.ports.length)} ports selected.`
                    : "Lists and ranges, for example 22, 80, 443, 8000-8100."
                }
              >
                <Field
                  id="settings-ports"
                  mono
                  value={settings.portSpec}
                  error={portCheck.error}
                  onChange={(e) => onChange({ portSpec: e.target.value })}
                />
              </FieldRow>

              <div className="grid grid-cols-2 gap-3">
                <NumberRow
                  id="settings-timeout"
                  label="Probe timeout"
                  suffix="ms"
                  value={settings.timeoutMs}
                  min={100}
                  max={10000}
                  step={50}
                  onChange={(timeoutMs) => onChange({ timeoutMs })}
                />
                <NumberRow
                  id="settings-hosts"
                  label="Host concurrency"
                  value={settings.hostConcurrency}
                  min={1}
                  max={1024}
                  step={1}
                  onChange={(hostConcurrency) => onChange({ hostConcurrency })}
                />
                <NumberRow
                  id="settings-tcp"
                  label="TCP probes"
                  value={settings.tcpConcurrency}
                  min={8}
                  max={2048}
                  step={8}
                  onChange={(tcpConcurrency) => onChange({ tcpConcurrency })}
                />
                <NumberRow
                  id="settings-ping"
                  label="Ping processes"
                  value={settings.pingConcurrency}
                  min={1}
                  max={128}
                  step={1}
                  onChange={(pingConcurrency) => onChange({ pingConcurrency })}
                />
              </div>
              <p className="text-xs leading-relaxed text-text-muted">
                These limits apply to the Custom and Full TCP profiles. The named profiles use their
                own so their scans stay comparable. ArcScan enforces its own ceilings on top of
                whatever you set here.
              </p>

              <div className="divider" />

              <div>
                <NumberRow
                  id="settings-retention"
                  label="Scans to keep"
                  value={settings.historyRetention}
                  min={5}
                  max={5000}
                  step={5}
                  onChange={(historyRetention) => onChange({ historyRetention })}
                />
                <p className="mt-1 text-xs leading-relaxed text-text-muted">
                  Older scans are removed after each new one. Device names, notes and first-seen
                  dates are kept whatever happens to the scans that recorded them
                  {portable ? " during this session." : "."}
                </p>
              </div>
              <Toggle
                id="settings-notify"
                label="Summarise changes after each scan"
                description="Shows a short notification when devices arrive, go missing or change."
                checked={settings.notifyOnChanges}
                onChange={(notifyOnChanges) => onChange({ notifyOnChanges })}
              />
            </Group>

            <Group
              id="settings-discovery"
              title="Discovery"
              lead="How ArcScan works out what each device is."
            >
              <p className="text-xs leading-relaxed text-text-secondary">
                Printers, TVs, routers, cameras and media devices announce themselves on the local
                network. ArcScan can ask, which is how it recognises more of what it finds. It sends
                multicast queries on your own network only, uses no credentials, and contacts no
                cloud service. A scan of a remote or routed target never sends them at all, and
                results stay on this computer.
              </p>

              <Toggle
                id="settings-local-discovery"
                label="Use local device discovery"
                description="Sends mDNS and SSDP queries on the network being scanned, and uses the replies to work out device names and types. Turning this off makes a scan behave exactly as it did before this feature existed."
                checked={settings.localDiscovery}
                onChange={(localDiscovery) => onChange({ localDiscovery })}
              />

              {settings.localDiscovery ? (
                <Toggle
                  id="settings-device-descriptions"
                  label="Read local device descriptions"
                  description="When a device advertises a description page over SSDP, ArcScan reads it for the manufacturer and model. It only ever connects to an address on the network being scanned — never to the internet, another subnet, or a redirect — and it fetches nothing else."
                  checked={settings.readDeviceDescriptions}
                  onChange={(readDeviceDescriptions) => onChange({ readDeviceDescriptions })}
                />
              ) : null}

              {/* No threshold, no weights, no classifier controls. The number
                  only means anything alongside the definition of a qualifying
                  miss, and exposing one without the other invites a person to
                  turn it down to one and then disbelieve the result. */}
              <p className="text-xs leading-relaxed text-text-muted">
                ArcScan keeps older discovery evidence for history, but stops relying on it after
                repeated completed scans no longer confirm it. Nothing is deleted: the device
                inspector still shows it, dated by how many discovery scans have missed it.
              </p>
            </Group>

            <Group
              id="settings-credentials"
              title="Credentials"
              lead="Held in memory for this session only, never written anywhere."
            >
              <WindowsCredentialSection />
              <p className="text-xs leading-relaxed text-text-muted">
                SNMP credentials for topology discovery are entered in the Scan view's Topology tab,
                and are kept for the session in the same way.
              </p>
            </Group>

            <Group
              id="settings-networks"
              title="Networks"
              lead="Each network keeps its own devices, names and notes."
            >
              {props.scopes.length > 0 ? (
                <>
                  <p className="text-xs leading-relaxed text-text-muted">
                    ArcScan keeps each network separate, so two sites that reuse the same addresses
                    can never mix. Name them here to tell them apart.
                  </p>
                  <ul className="space-y-2">
                    {props.scopes.map((scope) => (
                      <ScopeRow key={scope.id} scope={scope} onRename={props.onRenameScope} />
                    ))}
                  </ul>
                </>
              ) : (
                <p className="text-xs leading-relaxed text-text-muted">
                  A network appears here after its first saved scan, ready to be named.
                </p>
              )}
            </Group>

            <Group id="settings-appearance" title="Appearance" lead="How ArcScan looks, and what its tables show.">
              <div className="grid grid-cols-2 gap-3">
                <FieldRow label="Theme" htmlFor="settings-theme">
                  <Select
                    id="settings-theme"
                    value={settings.theme}
                    onChange={(e) => onChange({ theme: e.target.value as Settings["theme"] })}
                  >
                    <option value="system">Match the system</option>
                    <option value="light">Light</option>
                    <option value="dark">Dark</option>
                  </Select>
                </FieldRow>
                <FieldRow label="Row density" htmlFor="settings-density">
                  <Select
                    id="settings-density"
                    value={settings.density}
                    onChange={(e) => onChange({ density: e.target.value as Settings["density"] })}
                  >
                    <option value="compact">Compact</option>
                    <option value="comfortable">Comfortable</option>
                  </Select>
                </FieldRow>
              </div>
              <Toggle
                id="settings-motion"
                label="Reduced motion"
                description="Switches off row and panel animations. Applied automatically when your system already asks for reduced motion."
                checked={settings.reducedMotion}
                onChange={(reducedMotion) => onChange({ reducedMotion })}
              />

              <div>
                <h4 className="text-[13px] font-medium text-text">Scan results columns</h4>
                <p className="mt-0.5 text-xs leading-relaxed text-text-muted">
                  State, Name and IP address are always shown. Columns also hide themselves when the
                  window is too narrow for them.
                </p>
                <ul className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1.5">
                  {COLUMNS.filter((c) => !c.required).map((column) => (
                    <li key={column.key}>
                      <Toggle
                        id={`settings-column-${column.key}`}
                        label={column.label}
                        checked={!settings.hiddenColumns.includes(column.key)}
                        onChange={(visible) =>
                          onChange({
                            hiddenColumns: visible
                              ? settings.hiddenColumns.filter((k) => k !== column.key)
                              : ([...settings.hiddenColumns, column.key] as SortKey[]),
                          })
                        }
                      />
                    </li>
                  ))}
                </ul>
              </div>

              <div>
                <h4 className="text-[13px] font-medium text-text">Inventory columns</h4>
                <p className="mt-0.5 text-xs leading-relaxed text-text-muted">
                  Device, Address, Status and Last seen are always shown. These extras are off by
                  default and, like the rest, hide themselves when the window is too narrow.
                </p>
                <ul className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1.5">
                  {OPTIONAL_INVENTORY_COLUMNS.map((column) => (
                    <li key={column.key}>
                      <Toggle
                        id={`settings-inventory-column-${column.key}`}
                        label={column.label}
                        checked={settings.inventoryColumns.includes(column.key)}
                        onChange={(visible) =>
                          onChange({
                            inventoryColumns: visible
                              ? ([...settings.inventoryColumns, column.key] as InventoryColumn[])
                              : settings.inventoryColumns.filter((k) => k !== column.key),
                          })
                        }
                      />
                    </li>
                  ))}
                </ul>
              </div>

              <Toggle
                id="settings-guidance"
                label="Show first-run guidance"
                description="Brings back the explanation shown on the empty scan screen."
                checked={settings.showFirstRunGuidance}
                onChange={(showFirstRunGuidance) => onChange({ showFirstRunGuidance })}
              />
            </Group>

            <Group
              id="settings-application"
              title="Application"
              lead="This copy of ArcScan, its updates, and everything it may send beyond your network."
            >
              {props.runtime ? (
                <AboutSection
                  runtime={props.runtime}
                  onOpenDataFolder={props.onOpenDataFolder}
                  onOpenPortableDownloads={props.onOpenPortableDownloads}
                  native={props.native}
                />
              ) : null}

              <section className="space-y-3">
                <h4 className="text-[13px] font-medium text-text">Network requests</h4>
                <p className="text-xs leading-relaxed text-text-secondary">
                  Scanning is entirely local. ArcScan never sends your targets, results, device
                  names, MAC addresses or notes anywhere. These controls cover the only requests it
                  makes beyond your own network.
                </p>

                <Toggle
                  id="settings-public-ip"
                  label="Offer the public IP lookup"
                  description="Shows the Public IP utility on the Scan screen. It never looks anything up on its own: each check happens only when you press Check, and asks a third-party provider (api64.ipify.org, then icanhazip.com) for the address your internet connection appears from. Your targets, results and device details are never sent to them, and the answer is kept in memory for this session only."
                  checked={settings.publicIpLookup}
                  onChange={(publicIpLookup) => {
                    onChange({ publicIpLookup });
                    if (!publicIpLookup) props.onClearPublicIp();
                  }}
                />

                {/* The checking, copying and refreshing all live on the Scan
                    screen, so this is the session value and one way to forget
                    it, not a second set of controls competing with the first. */}
                {settings.publicIpLookup && props.publicIp.status === "ready" ? (
                  <div className="flex items-center justify-between gap-2 rounded-md border border-border bg-surface-sunken px-2.5 py-2">
                    <div className="min-w-0">
                      <p className="text-xs text-text-muted">This session</p>
                      <p className="mono truncate text-[13px] text-text">{props.publicIp.ip}</p>
                    </div>
                    <Button size="sm" variant="ghost" onClick={props.onClearPublicIp}>
                      Forget
                    </Button>
                  </div>
                ) : null}

                {props.runtime?.updater_mode === "installer" ? (
                  <Toggle
                    id="settings-updates"
                    label="Check for updates on launch"
                    description="Asks GitHub whether a newer signed release exists. Only the version being checked is sent."
                    checked={settings.checkForUpdates}
                    onChange={(checkForUpdates) => onChange({ checkForUpdates })}
                  />
                ) : null}

                <Button
                  size="sm"
                  variant="ghost"
                  icon={<ExternalLink className="h-3.5 w-3.5" />}
                  onClick={props.onOpenPrivacy}
                >
                  Read the full privacy notes
                </Button>
              </section>

              {props.arcAtlas && props.onManageArcAtlas ? (
                <section className="space-y-2">
                  <h4 className="text-[13px] font-medium text-text">ArcAtlas</h4>
                  <div className="flex items-center justify-between gap-3 rounded-md border border-border bg-surface-sunken px-3 py-2">
                    <p className="min-w-0 text-xs leading-relaxed text-text-secondary">
                      {props.arcAtlas.configured && !props.arcAtlas.needsReconfigure ? (
                        <>
                          Connected to{" "}
                          <span className="font-medium text-text">
                            {destinationLabel(props.arcAtlas)}
                          </span>
                          . A completed scan can be sent from the Scan view or the Inventory.
                        </>
                      ) : props.arcAtlas.needsReconfigure ? (
                        "The ArcAtlas connection needs to be set up again."
                      ) : (
                        "Not connected. Connect once to send a network's inventory to ArcAtlas after a scan."
                      )}
                    </p>
                    <Button size="sm" onClick={props.onManageArcAtlas}>
                      {props.arcAtlas.configured && !props.arcAtlas.needsReconfigure
                        ? "Manage connection"
                        : "Connect"}
                    </Button>
                  </div>
                </section>
              ) : null}
            </Group>
          </div>
        </div>
      </Drawer>

      <ConfirmDialog
        open={confirmReset}
        title="Reset all settings?"
        description={
          portable
            ? "Every preference goes back to its default. Inventory, History, Changes, names and notes in this session are not touched."
            : "Every preference goes back to its default. Your scan history, device names and notes are not touched."
        }
        confirmLabel="Reset settings"
        onCancel={() => setConfirmReset(false)}
        onConfirm={() => {
          setConfirmReset(false);
          props.onReset();
        }}
      />
    </>
  );
}

/** One settings group: a heading, one line on what it covers, its controls. */
function Group({
  id,
  title,
  lead,
  children,
}: {
  id: string;
  title: string;
  lead: string;
  children: React.ReactNode;
}) {
  return (
    <section id={id} aria-labelledby={`${id}-title`} className="scroll-mt-2 border-b border-border py-5 last:border-b-0">
      <h3 id={`${id}-title`} className="text-sm font-semibold text-text">
        {title}
      </h3>
      <p className="mt-0.5 text-xs text-text-muted">{lead}</p>
      <div className="mt-4 space-y-4">{children}</div>
    </section>
  );
}

/**
 * Which edition this is, and whether it keeps data after this process.
 *
 * Deliberately understated. An installed ArcScan has always kept its data
 * somewhere the operator never had to think about, and it still does. Portable
 * deliberately withholds its internal temp path: showing it would invite an
 * export into a directory ArcScan is required to remove on exit.
 */
function AboutSection({
  runtime,
  native,
  onOpenDataFolder,
  onOpenPortableDownloads,
}: {
  runtime: RuntimeInfo;
  native: boolean;
  onOpenDataFolder: () => void;
  onOpenPortableDownloads: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const portable = runtime.edition === "portable";

  const copy = async () => {
    if (!runtime.data_root) return;
    try {
      await navigator.clipboard.writeText(runtime.data_root);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // A clipboard the browser or the platform refuses is not worth an error
      // dialog: the path is on screen and selectable.
    }
  };

  return (
    <>
      <section>
        <h4 className="text-[13px] font-medium text-text">About this copy</h4>
        <p className="text-[13px] text-text">ArcScan {version(runtime.version)}</p>
        <p className="mt-0.5 text-[12px] text-text-secondary" data-testid="edition-label">
          {editionLabel(runtime)}
        </p>

        {portable ? (
          <div className="mt-3" data-testid="portable-session-storage">
            <p className="text-xs text-text-muted">Session storage</p>
            <p className="mt-1 text-[13px] font-medium text-text">Temporary</p>
            <p className="mt-1 text-[12px] leading-relaxed text-text-secondary">
              This launch starts fresh. Inventory, History, Changes, names, notes, discovery
              evidence and preferences are removed when ArcScan closes. Export CSV, JSON or XML
              outside ArcScan if you want to keep it.
            </p>
          </div>
        ) : runtime.data_root ? (
          <div className="mt-3">
            <p className="text-xs text-text-muted">Data location</p>
            <p
              className="mono mt-1 break-all rounded-md border border-border bg-surface-sunken px-2.5 py-2 text-[12px] text-text"
              data-testid="data-root"
            >
              {runtime.data_root}
            </p>
            <div className="mt-2 flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="ghost"
                icon={
                  copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />
                }
                onClick={() => void copy()}
              >
                {copied ? "Copied" : "Copy data path"}
              </Button>
              {native ? (
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<FolderOpen className="h-3.5 w-3.5" />}
                  onClick={onOpenDataFolder}
                >
                  Open data folder
                </Button>
              ) : null}
            </div>
          </div>
        ) : null}

        {portable ? (
          <div className="mt-3">
            <p className="text-[12px] leading-relaxed text-text-secondary">
              Portable ArcScan cannot apply the Installer update. Export anything you want to keep,
              finish this session and close ArcScan, then download and extract the latest Portable
              ZIP.
            </p>
            <Button
              size="sm"
              variant="ghost"
              className="mt-2"
              icon={<ExternalLink className="h-3.5 w-3.5" />}
              onClick={onOpenPortableDownloads}
            >
              View Portable downloads
            </Button>
          </div>
        ) : null}
      </section>
    </>
  );
}

function version(v: string): string {
  return v.startsWith("v") ? v : `v${v}`;
}

/** One network scope with an inline rename field, saved on blur or Enter. */
function ScopeRow({
  scope,
  onRename,
}: {
  scope: NetworkScope;
  onRename: (id: number, name: string) => void;
}) {
  const [draft, setDraft] = useState(scope.display_name);
  return (
    <li className="rounded-md border border-border bg-surface-sunken px-2.5 py-2">
      <Field
        aria-label={`Name for ${scope.canonical_target ?? scope.display_name}`}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => {
          const next = draft.trim();
          if (next && next !== scope.display_name) onRename(scope.id, next);
          else setDraft(scope.display_name);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
        }}
      />
      <p className="mono mt-1 text-xs text-text-muted">
        {scope.canonical_target?.replace(/^(cidr|range|host):/, "") ?? "Earlier inventory"}
        {scope.gateway_mac ? ` · gateway ${scope.gateway_mac}` : ""}
        {" · "}
        {scope.device_count} {scope.device_count === 1 ? "device" : "devices"}
      </p>
    </li>
  );
}

/** A labelled switch. A real button with role="switch", not a styled checkbox. */
function Toggle({
  id,
  label,
  description,
  checked,
  onChange,
  className = "",
}: {
  id: string;
  label: string;
  description?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  className?: string;
}) {
  return (
    <div className={`flex items-start gap-2.5 ${className}`}>
      <button
        type="button"
        id={id}
        role="switch"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        className="relative mt-0.5 h-[18px] w-8 shrink-0 rounded-full transition-colors duration-base"
        style={{ background: checked ? "var(--accent)" : "var(--border-strong)" }}
      >
        <span
          className="absolute left-0.5 top-0.5 h-[14px] w-[14px] rounded-full bg-white shadow-sm transition-transform duration-base"
          style={{ transform: checked ? "translateX(14px)" : "translateX(0)" }}
        />
      </button>
      <label htmlFor={id} className="min-w-0 cursor-pointer select-none">
        <span className="block text-[13px] font-medium text-text">{label}</span>
        {description ? (
          <span className="mt-0.5 block text-xs leading-relaxed text-text-muted">{description}</span>
        ) : null}
      </label>
    </div>
  );
}

function NumberRow({
  id,
  label,
  value,
  min,
  max,
  step,
  suffix,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  suffix?: string;
  onChange: (value: number) => void;
}) {
  return (
    <div>
      <label className="field-label" htmlFor={id}>
        {label}
        {suffix ? <span className="text-text-muted"> ({suffix})</span> : null}
      </label>
      <Field
        id={id}
        type="number"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => {
          const next = Number(event.target.value);
          // Clamp on change rather than on blur, so a pasted value out of range
          // never reaches a scan.
          if (Number.isFinite(next)) onChange(Math.min(max, Math.max(min, next)));
        }}
      />
    </div>
  );
}

/**
 * The Windows credential a credentialed deep scan uses.
 *
 * # What this section promises, and how it keeps it
 *
 * The password is typed here, sent to the backend once, and held in process
 * memory until ArcScan closes. It is not written to SQLite, to the system
 * keyring, to a configuration file or to `localStorage`; it is not sent to
 * ArcAtlas; it does not appear in a diagnostic report or an export. There is no
 * command that reads it back, so nothing on this screen — or anywhere else in
 * the interface — can display it.
 *
 * The local field is cleared the moment the credential is handed over, so the
 * password does not sit in a React state tree for the rest of the session
 * either.
 *
 * One credential, typed by a person. ArcScan has no list of accounts to try, no
 * fallback, and no retry with a different password: a machine that refuses the
 * credential is recorded as having refused it and left alone, which is what
 * stops a scan from locking out a domain account.
 */
function WindowsCredentialSection() {
  const [status, setStatus] = useState<WindowsCredentialStatus | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Subscribed, so this panel and the command bar's depth picker can never
  // disagree about whether a credential is set.
  useEffect(() => {
    const unsubscribe = windowsCredentials.subscribe(setStatus);
    void windowsCredentials.refresh().then((next) => {
      if (!next) {
        setStatus({
          configured: false,
          account: null,
          supported: false,
          unsupported_reason: "ArcScan could not reach its credential store.",
        });
      }
    });
    return unsubscribe;
  }, []);

  const supported = status?.supported !== false;

  async function save(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      // Through the store, so every other reader is told at the same moment.
      await windowsCredentials.set(username, null, password);
      // Cleared immediately: there is no reason for the password to stay in a
      // React state tree once the backend has it.
      setPassword("");
      setUsername("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function clear() {
    setBusy(true);
    setError(null);
    try {
      await windowsCredentials.clear();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <h4 className="mb-2 text-[13px] font-medium text-text">Windows credential</h4>
      <p className="mb-3 text-xs leading-relaxed text-text-secondary">
        A credentialed deep scan asks Windows machines about themselves, which is the only way to
        establish an exact edition, version and build — and the only way to tell a workstation from
        a server with certainty rather than by guessing from open ports.
      </p>
      <p className="mb-3 text-xs leading-relaxed text-text-secondary">
        The password is kept in memory for this session only. It is never written to disk, never
        sent to ArcAtlas, never logged, and never included in a diagnostic report or an export.
        Closing ArcScan forgets it. ArcScan tries the one account you give it and nothing else: a
        machine that refuses it is recorded as having refused it and left alone.
      </p>

      {!supported ? (
        <p className="rounded-md border border-border bg-surface-sunken px-3 py-2 text-xs leading-relaxed text-text-muted">
          {status?.unsupported_reason ??
            "Credentialed Windows discovery is unavailable in this build."}
        </p>
      ) : status?.configured ? (
        <div className="flex items-center justify-between gap-3 rounded-md border border-border bg-surface-sunken px-3 py-2">
          <span className="flex min-w-0 items-center gap-2 text-[13px]">
            <KeyRound className="h-3.5 w-3.5 shrink-0 text-text-muted" aria-hidden />
            <span className="truncate font-mono text-text">{status.account}</span>
          </span>
          <Button onClick={clear} busy={busy} title="Forget this credential">
            Forget
          </Button>
        </div>
      ) : (
        <form onSubmit={save} className="space-y-3">
          <FieldRow
            label="User name"
            htmlFor="settings-windows-user"
            hint="DOMAIN\\user, user@domain, or a local account name."
          >
            <Field
              id="settings-windows-user"
              mono
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              placeholder="CORP\\svc-arcscan"
              autoComplete="off"
            />
          </FieldRow>
          <FieldRow label="Password" htmlFor="settings-windows-password" error={error}>
            <Field
              id="settings-windows-password"
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              autoComplete="off"
              error={error}
            />
          </FieldRow>
          <Button
            type="submit"
            variant="primary"
            busy={busy}
            disabled={!username.trim() || !password}
          >
            Use for this session
          </Button>
        </form>
      )}
    </section>
  );
}
