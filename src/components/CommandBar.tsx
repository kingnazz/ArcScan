// The scan command bar.
//
// One row, with a clear hierarchy: the target field and the Scan action dominate,
// the profile picker sits directly beside them because it changes what Scan does,
// and everything else is secondary. The advanced controls live in a popover so
// they are one click away without competing for attention.

import { forwardRef, useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, Crosshair, Play, RotateCw, Settings2, Square } from "lucide-react";
import { Button, Field, FieldRow, IconButton } from "../ui/primitives";
import { Popover } from "../ui/Popover";
import { api } from "../lib/api";
import { formatCount, parsePorts } from "../lib/format";
import {
  PROFILES,
  PROFILE_ORDER,
  buildScanOptions,
  type ProfileId,
} from "../lib/profiles";
import type { LocalNetwork, ScanOptions, ScanPreview } from "../types";
import type { Settings } from "../lib/prefs";

export interface CommandBarProps {
  scanning: boolean;
  stopping: boolean;
  target: string;
  onTargetChange: (target: string) => void;
  profileId: ProfileId;
  onProfileChange: (id: ProfileId) => void;
  settings: Settings;
  onSettingsChange: (patch: Partial<Settings>) => void;
  recents: string[];
  localNetworks: LocalNetwork[];
  canRescan: boolean;
  onScan: (opts: ScanOptions) => void;
  onStop: () => void;
  onRescan: () => void;
  onError: (message: string) => void;
}

export const CommandBar = forwardRef<HTMLInputElement, CommandBarProps>(function CommandBar(
  {
    scanning,
    stopping,
    target,
    onTargetChange,
    profileId,
    onProfileChange,
    settings,
    onSettingsChange,
    recents,
    localNetworks,
    canRescan,
    onScan,
    onStop,
    onRescan,
    onError,
  },
  targetRef,
) {
  const advancedButton = useRef<HTMLButtonElement>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [preview, setPreview] = useState<ScanPreview | null>(null);

  const portSpec = settings.portSpec;
  const parsed = useMemo(() => parsePorts(portSpec), [portSpec]);
  const tunable = profileId === "custom" || profileId === "full-tcp";

  const options = useMemo(
    () =>
      buildScanOptions(target, profileId, {
        ports: parsed.ports,
        timeout_ms: settings.timeoutMs,
        concurrency: settings.hostConcurrency,
        tcp_concurrency: settings.tcpConcurrency,
        ping_concurrency: settings.pingConcurrency,
      }),
    [
      target,
      profileId,
      parsed.ports,
      settings.timeoutMs,
      settings.hostConcurrency,
      settings.tcpConcurrency,
      settings.pingConcurrency,
    ],
  );

  // Ask the backend what this scan would do, so the workload is visible before
  // the operator commits. Debounced, because it runs on every keystroke.
  useEffect(() => {
    if (!target.trim()) {
      setPreview(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      api
        .previewScan(options)
        .then((result) => {
          if (!cancelled) setPreview(result);
        })
        .catch(() => {
          // An invalid target is normal while typing. The real error is reported
          // when Scan is pressed, against the backend's own validation.
          if (!cancelled) setPreview(null);
        });
    }, 220);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [options, target]);

  const canScan = target.trim().length > 0 && !scanning && !parsed.error;

  function submit(event: React.FormEvent) {
    event.preventDefault();
    if (scanning) return;
    if (parsed.error) {
      onError(parsed.error);
      return;
    }
    if (!target.trim()) {
      onError("Enter a target: a single IP, a dashed range, or a CIDR block.");
      return;
    }
    onScan(options);
  }

  const profile = PROFILES[profileId];

  return (
    <form
      onSubmit={submit}
      className="flex items-center gap-2 border-b border-border bg-surface px-3 py-2"
    >
      <ProfilePicker value={profileId} onChange={onProfileChange} disabled={scanning} />

      <div className="min-w-0 flex-1">
        <Field
          ref={targetRef}
          id="scan-target"
          scale="lg"
          mono
          value={target}
          onChange={(event) => onTargetChange(event.target.value)}
          placeholder="192.168.1.0/24    10.0.0.1-50    192.168.1.20"
          aria-label="Target: a single IP address, a dashed range, or a CIDR block"
          autoComplete="off"
          list="arcscan-recent-targets"
        />
        {recents.length > 0 ? (
          <datalist id="arcscan-recent-targets">
            {recents.map((r) => (
              <option key={r} value={r} />
            ))}
          </datalist>
        ) : null}
      </div>

      {scanning ? (
        <Button
          size="lg"
          variant="danger"
          className="min-w-[104px]"
          busy={stopping}
          icon={<Square className="h-3.5 w-3.5 fill-current" />}
          onClick={onStop}
          title="Stop the scan and keep what was found (Escape)"
        >
          {stopping ? "Stopping" : "Stop"}
        </Button>
      ) : (
        <Button
          type="submit"
          size="lg"
          variant="primary"
          className="min-w-[104px]"
          disabled={!canScan}
          icon={<Play className="h-3.5 w-3.5" />}
          title="Start the scan (Enter)"
        >
          Scan
        </Button>
      )}

      <IconButton
        label="Rescan the last target"
        onClick={onRescan}
        disabled={!canRescan || scanning}
      >
        <RotateCw className="h-4 w-4" />
      </IconButton>

      <IconButton
        label="Use this computer's network"
        onClick={() => {
          const first = localNetworks[0];
          if (first) onTargetChange(first.cidr);
          else onError("ArcScan could not detect a local network on this computer.");
        }}
        disabled={scanning || localNetworks.length === 0}
      >
        <Crosshair className="h-4 w-4" />
      </IconButton>

      <div className="divider-v mx-0.5 my-1" aria-hidden />

      <div className="relative">
        <Button
          ref={advancedButton}
          icon={<Settings2 className="h-3.5 w-3.5" />}
          aria-expanded={advancedOpen}
          aria-haspopup="dialog"
          onClick={() => setAdvancedOpen((v) => !v)}
          title="Ports, timeout and concurrency for this scan"
        >
          Advanced
        </Button>
        <Popover
          open={advancedOpen}
          onClose={() => setAdvancedOpen(false)}
          anchor={advancedButton}
          label="Advanced scan settings"
          className="w-[22rem] p-3"
        >
          <AdvancedPanel
            profileId={profileId}
            tunable={tunable}
            settings={settings}
            onSettingsChange={onSettingsChange}
            portError={parsed.error}
            portCount={parsed.ports.length}
            preview={preview}
          />
        </Popover>
      </div>

      <p className="ml-1 hidden max-w-[16rem] shrink-0 truncate text-xs text-text-muted xl:block">
        {preview ? (
          <>
            <span className="font-medium text-text-secondary">{formatCount(preview.total)}</span>{" "}
            {preview.total === 1 ? "address" : "addresses"} ·{" "}
            <span className="font-medium text-text-secondary">{preview.port_count}</span> ports ·{" "}
            {profile.name}
          </>
        ) : (
          profile.summary
        )}
      </p>
    </form>
  );
});

function ProfilePicker({
  value,
  onChange,
  disabled,
}: {
  value: ProfileId;
  onChange: (id: ProfileId) => void;
  disabled: boolean;
}) {
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const profile = PROFILES[value];

  return (
    <div className="relative shrink-0">
      <button
        ref={button}
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        title="Scan profile"
        className="btn btn-secondary btn-lg w-[9.5rem] justify-between disabled:opacity-45"
      >
        <span className="truncate">{profile.name}</span>
        <ChevronDown className="h-3.5 w-3.5 shrink-0 opacity-60" aria-hidden />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={button}
        align="start"
        label="Scan profile"
        className="w-[21rem] p-1"
      >
        <ul role="listbox" aria-label="Scan profile">
          {PROFILE_ORDER.map((id) => {
            const item = PROFILES[id];
            const selected = id === value;
            return (
              <li key={id}>
                <button
                  type="button"
                  role="option"
                  aria-selected={selected}
                  onClick={() => {
                    onChange(id);
                    setOpen(false);
                    button.current?.focus();
                  }}
                  className={`w-full rounded-md px-2.5 py-2 text-left transition-colors duration-fast hover:bg-surface-hover ${
                    selected ? "bg-accent-subtle" : ""
                  }`}
                >
                  <span className="flex items-baseline gap-2">
                    <span className="text-[13px] font-medium text-text">{item.name}</span>
                    {selected ? (
                      <span className="text-[10.5px] font-semibold uppercase tracking-wide text-accent-text">
                        Selected
                      </span>
                    ) : null}
                  </span>
                  <span className="mt-0.5 block text-xs leading-relaxed text-text-secondary">
                    {item.detail}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </Popover>
    </div>
  );
}

function AdvancedPanel({
  profileId,
  tunable,
  settings,
  onSettingsChange,
  portError,
  portCount,
  preview,
}: {
  profileId: ProfileId;
  tunable: boolean;
  settings: Settings;
  onSettingsChange: (patch: Partial<Settings>) => void;
  portError: string | null;
  portCount: number;
  preview: ScanPreview | null;
}) {
  const profile = PROFILES[profileId];

  if (!tunable) {
    return (
      <div className="space-y-3">
        <p className="text-[13px] leading-relaxed text-text-secondary">
          <span className="font-medium text-text">{profile.name}</span> sets its own ports, timeout
          and concurrency limits so its scans stay comparable with each other.
        </p>
        <dl className="space-y-1 text-xs">
          <PanelStat label="Ports" value={`${profile.ports.length} selected`} />
          <PanelStat label="Timeout" value={`${profile.timeout_ms} ms per probe`} />
          <PanelStat label="Host concurrency" value={String(profile.concurrency)} />
          <PanelStat label="TCP probes" value={`${profile.tcp_concurrency} at once`} />
          <PanelStat label="Ping processes" value={`${profile.ping_concurrency} at once`} />
        </dl>
        <p className="text-xs leading-relaxed text-text-muted">
          Switch to the Custom profile to set these yourself.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3.5">
      <FieldRow
        label="TCP ports"
        htmlFor="advanced-ports"
        error={portError}
        hint={
          portCount > 0
            ? `${formatCount(portCount)} ports selected. Lists and ranges, for example 22, 80, 443, 8000-8100.`
            : "Lists and ranges, for example 22, 80, 443, 8000-8100."
        }
      >
        <Field
          id="advanced-ports"
          mono
          value={settings.portSpec}
          onChange={(event) => onSettingsChange({ portSpec: event.target.value })}
          placeholder="22, 80, 443, 3389, 8000-8100"
          error={portError}
        />
      </FieldRow>

      <SliderRow
        id="advanced-timeout"
        label="Probe timeout"
        value={settings.timeoutMs}
        min={100}
        max={4000}
        step={50}
        format={(v) => `${v} ms`}
        hint="Longer timeouts find more slow devices and take longer."
        onChange={(timeoutMs) => onSettingsChange({ timeoutMs })}
      />
      <SliderRow
        id="advanced-hosts"
        label="Host concurrency"
        value={settings.hostConcurrency}
        min={1}
        max={512}
        step={1}
        format={String}
        hint="Addresses worked on at once."
        onChange={(hostConcurrency) => onSettingsChange({ hostConcurrency })}
      />
      <SliderRow
        id="advanced-tcp"
        label="TCP probes"
        value={settings.tcpConcurrency}
        min={8}
        max={1024}
        step={8}
        format={String}
        hint="Connection attempts in flight across the whole scan. Higher values can make consumer routers drop replies."
        onChange={(tcpConcurrency) => onSettingsChange({ tcpConcurrency })}
      />
      <SliderRow
        id="advanced-ping"
        label="Ping processes"
        value={settings.pingConcurrency}
        min={1}
        max={128}
        step={1}
        format={String}
        hint="Ping runs as a child process, which costs far more than a socket."
        onChange={(pingConcurrency) => onSettingsChange({ pingConcurrency })}
      />

      {preview?.warning ? (
        <p
          role="status"
          className="rounded-md border border-warning/40 bg-warning-subtle px-2.5 py-2 text-xs leading-relaxed text-warning"
        >
          {preview.warning}
        </p>
      ) : null}
    </div>
  );
}

function PanelStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-3">
      <dt className="text-text-muted">{label}</dt>
      <dd className="font-medium text-text-secondary">{value}</dd>
    </div>
  );
}

function SliderRow({
  id,
  label,
  value,
  min,
  max,
  step,
  format,
  hint,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (v: number) => string;
  hint: string;
  onChange: (value: number) => void;
}) {
  return (
    <div>
      <div className="mb-1 flex items-baseline justify-between gap-2">
        <label className="field-label mb-0" htmlFor={id}>
          {label}
        </label>
        <span className="mono text-xs font-semibold text-text">{format(value)}</span>
      </div>
      <input
        id={id}
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
        className="w-full accent-accent"
        aria-describedby={`${id}-hint`}
      />
      <p id={`${id}-hint`} className="mt-1 text-xs leading-relaxed text-text-muted">
        {hint}
      </p>
    </div>
  );
}
