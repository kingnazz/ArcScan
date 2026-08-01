// Settings.
//
// Global preferences live here rather than on the scanning screen, so the scan
// surface stays about the scan. Grouped by what the operator is trying to change,
// with the privacy-relevant switches stated plainly rather than buried.

import { useState } from "react";
import { ExternalLink, Globe, RotateCcw } from "lucide-react";
import { Button, Field, FieldRow, SectionHeading, Select } from "../ui/primitives";
import { Drawer } from "../ui/Drawer";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { PROFILES, PROFILE_ORDER } from "../lib/profiles";
import { COLUMNS, type SortKey } from "../lib/table";
import { formatCount, parsePorts } from "../lib/format";
import type { Settings } from "../lib/prefs";
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
  publicIp: PublicIpState;
  onCheckPublicIp: () => void;
  onClearPublicIp: () => void;
  onCopyPublicIp: (ip: string) => void;
  onOpenPrivacy: () => void;
  /** Known network scopes; empty until a scan has been saved. */
  scopes: NetworkScope[];
  onRenameScope: (id: number, name: string) => void;
}

export function SettingsPanel(props: SettingsPanelProps) {
  const { settings, onChange } = props;
  const [confirmReset, setConfirmReset] = useState(false);
  const portCheck = parsePorts(settings.portSpec);

  return (
    <>
      <Drawer
        open={props.open}
        onClose={props.onClose}
        overlay={props.overlay}
        width={props.width}
        onWidthChange={props.onWidthChange}
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
        <div className="space-y-5">
          <section>
            <SectionHeading>Appearance</SectionHeading>
            <div className="space-y-3">
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
                  <option value="compact">Compact, more rows on screen</option>
                  <option value="comfortable">Comfortable, more space per row</option>
                </Select>
              </FieldRow>
              <Toggle
                id="settings-motion"
                label="Reduced motion"
                description="Switches off row and panel animations. Applied automatically when your system already asks for reduced motion."
                checked={settings.reducedMotion}
                onChange={(reducedMotion) => onChange({ reducedMotion })}
              />
            </div>
          </section>

          <div className="divider" />

          <section>
            <SectionHeading>Scanning</SectionHeading>
            <div className="space-y-3">
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
            </div>
          </section>

          <div className="divider" />

          <section>
            <SectionHeading>Results</SectionHeading>
            <p className="mb-2 text-xs leading-relaxed text-text-muted">
              Name, IP address and state are always shown. Columns also hide themselves
              automatically when the window is too narrow for them.
            </p>
            <ul className="space-y-1.5">
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
          </section>

          <div className="divider" />

          <section>
            <SectionHeading>History</SectionHeading>
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
              Older scans are removed after each new one. Device names, notes and first-seen dates
              are kept whatever happens to the scans that recorded them.
            </p>
            <Toggle
              id="settings-notify"
              className="mt-3"
              label="Summarise changes after each scan"
              description="Shows a short notification when devices arrive, go missing or change."
              checked={settings.notifyOnChanges}
              onChange={(notifyOnChanges) => onChange({ notifyOnChanges })}
            />
          </section>

          {props.scopes.length > 0 ? (
            <>
              <div className="divider" />
              <section>
                <SectionHeading>Networks</SectionHeading>
                <p className="mb-2 text-xs leading-relaxed text-text-muted">
                  ArcScan keeps each network's devices, names and notes separate, so two sites that
                  reuse the same addresses can never mix. Name them here to tell them apart.
                </p>
                <ul className="space-y-2">
                  {props.scopes.map((scope) => (
                    <ScopeRow key={scope.id} scope={scope} onRename={props.onRenameScope} />
                  ))}
                </ul>
              </section>
            </>
          ) : null}

          <div className="divider" />

          <section>
            <SectionHeading>Network requests</SectionHeading>
            <p className="mb-3 text-xs leading-relaxed text-text-secondary">
              Scanning is entirely local. ArcScan never sends your targets, results, device names,
              MAC addresses or notes anywhere. These two switches control the only requests it makes
              beyond your own network.
            </p>

            <Toggle
              id="settings-public-ip"
              label="Allow public IP lookups"
              description="Enables the Check public IP action, which asks a third-party service (api64.ipify.org, then icanhazip.com) for the address your internet connection appears from. Nothing else is sent, and the result is kept for this session only."
              checked={settings.publicIpLookup}
              onChange={(publicIpLookup) => {
                onChange({ publicIpLookup });
                if (!publicIpLookup) props.onClearPublicIp();
              }}
            />

            {settings.publicIpLookup ? (
              <div className="mt-2.5 rounded-md border border-border bg-surface-sunken px-2.5 py-2">
                {props.publicIp.status === "ready" ? (
                  <div className="flex items-center justify-between gap-2">
                    <span className="mono text-[13px] text-text">{props.publicIp.ip}</span>
                    <div className="flex gap-1.5">
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() =>
                          props.publicIp.status === "ready" && props.onCopyPublicIp(props.publicIp.ip)
                        }
                      >
                        Copy
                      </Button>
                      <Button size="sm" variant="ghost" onClick={props.onClearPublicIp}>
                        Forget
                      </Button>
                    </div>
                  </div>
                ) : props.publicIp.status === "error" ? (
                  <div className="space-y-2">
                    <p className="text-xs text-danger">{props.publicIp.message}</p>
                    <Button size="sm" onClick={props.onCheckPublicIp}>
                      Try again
                    </Button>
                  </div>
                ) : (
                  <Button
                    size="sm"
                    icon={<Globe className="h-3.5 w-3.5" />}
                    busy={props.publicIp.status === "checking"}
                    onClick={props.onCheckPublicIp}
                  >
                    Check public IP
                  </Button>
                )}
              </div>
            ) : null}

            <Toggle
              id="settings-updates"
              className="mt-3"
              label="Check for updates on launch"
              description="Asks GitHub whether a newer signed release exists. Only the version being checked is sent."
              checked={settings.checkForUpdates}
              onChange={(checkForUpdates) => onChange({ checkForUpdates })}
            />

            <Button
              size="sm"
              variant="ghost"
              className="mt-3"
              icon={<ExternalLink className="h-3.5 w-3.5" />}
              onClick={props.onOpenPrivacy}
            >
              Read the full privacy notes
            </Button>
          </section>

          <div className="divider" />

          <section>
            <SectionHeading>Getting started</SectionHeading>
            <Toggle
              id="settings-guidance"
              label="Show first-run guidance"
              description="Brings back the explanation shown on the empty scan screen."
              checked={settings.showFirstRunGuidance}
              onChange={(showFirstRunGuidance) => onChange({ showFirstRunGuidance })}
            />
          </section>
        </div>
      </Drawer>

      <ConfirmDialog
        open={confirmReset}
        title="Reset all settings?"
        description="Every preference goes back to its default. Your scan history, device names and notes are not touched."
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
