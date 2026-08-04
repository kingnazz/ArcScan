// The network and utility context row, directly under the view switcher.
//
// It answers two questions that belong together and that nothing else on the
// Scan screen answers: which network this machine is on, and — only if asked —
// which address the internet sees it from. Both are context for the scan below,
// so the row is deliberately quieter than the target field and the Scan button
// under it.

import { useEffect, useRef, useState } from "react";
import { AlertTriangle, Check, Copy, Globe, Network, RotateCw } from "lucide-react";
import { Button, IconButton, Spinner } from "../ui/primitives";
import { formatRelative } from "../lib/format";
import type { PublicIpState } from "../hooks/usePublicIp";
import type { LocalNetwork } from "../types";

export interface ContextBarProps {
  localNetworks: LocalNetwork[];
  publicIp: PublicIpState;
  /** Settings' "Offer the public IP lookup". Off hides the utility entirely. */
  publicIpEnabled: boolean;
  onCheckPublicIp: () => void;
  onCopyPublicIp: (ip: string) => void;
}

export function ContextBar({
  localNetworks,
  publicIp,
  publicIpEnabled,
  onCheckPublicIp,
  onCopyPublicIp,
}: ContextBarProps) {
  const [showTechnical, setShowTechnical] = useState(false);
  const technical = publicIp.status === "error" ? publicIp.technical : undefined;

  // A failed lookup that is retried should not keep the previous failure's
  // details open underneath the new state.
  useEffect(() => {
    if (publicIp.status !== "error") setShowTechnical(false);
  }, [publicIp.status]);

  const primary = localNetworks[0];

  // With no network detected and the lookup turned off there is nothing to say,
  // and an empty strip is worse than no strip.
  if (!primary && !publicIpEnabled) return null;

  return (
    <div className="shrink-0 border-b border-border bg-surface">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-1.5 text-xs">
        {primary ? (
          <p className="flex min-w-0 items-center gap-1.5">
            <Network className="h-3.5 w-3.5 shrink-0 text-text-muted" aria-hidden />
            <span className="shrink-0 text-text-muted">Local network</span>
            <span className="mono truncate text-text-secondary">{primary.cidr}</span>
            {localNetworks.length > 1 ? (
              <span className="shrink-0 text-text-muted">
                +{localNetworks.length - 1} more
              </span>
            ) : null}
          </p>
        ) : null}

        <div className="min-w-0 flex-1" />

        {publicIpEnabled ? (
          <PublicIp
            state={publicIp}
            onCheck={onCheckPublicIp}
            onCopy={onCopyPublicIp}
            technicalOpen={showTechnical}
            onToggleTechnical={technical ? () => setShowTechnical((v) => !v) : undefined}
          />
        ) : null}
      </div>

      {publicIpEnabled && showTechnical && technical ? (
        <pre className="mono mx-3 mb-2 max-h-24 overflow-auto whitespace-pre-wrap rounded border border-border bg-surface-sunken p-2 text-[11px] leading-relaxed text-text-secondary">
          {technical}
        </pre>
      ) : null}
    </div>
  );
}

/**
 * The Public IP utility.
 *
 * Nothing here runs on mount, on a view change, after a scan or on a timer. The
 * single action button is the only thing that starts a lookup, and it stays
 * mounted across every state so that pressing it never moves keyboard focus to
 * the body while the state changes underneath.
 */
function PublicIp({
  state,
  onCheck,
  onCopy,
  technicalOpen,
  onToggleTechnical,
}: {
  state: PublicIpState;
  onCheck: () => void;
  onCopy: (ip: string) => void;
  technicalOpen: boolean;
  onToggleTechnical?: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => () => clearTimeout(copyTimer.current), []);

  // The action keeps one slot in the DOM in every state, so React reuses the
  // same button element and focus survives idle -> checking -> ready.
  const action = {
    idle: { label: "Check public IP", text: "Check" },
    checking: { label: "Checking the public IP", text: "Checking" },
    ready: { label: "Check the public IP again", text: "Refresh" },
    error: { label: "Try the public IP lookup again", text: "Retry" },
  }[state.status];

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
      {/* Announced when it changes, so a lookup's result does not arrive
          silently for anyone not watching this corner of the window. */}
      <p role="status" className="flex min-w-0 items-center gap-1.5">
        {state.status === "error" ? (
          <>
            <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-danger" aria-hidden />
            <span className="font-medium text-text">Public IP unavailable</span>
            <span className="truncate text-text-secondary">
              Check your connection and try again.
            </span>
          </>
        ) : (
          <>
            <Globe className="h-3.5 w-3.5 shrink-0 text-text-muted" aria-hidden />
            <span className="shrink-0 text-text-muted">Public IP</span>
            {state.status === "idle" ? (
              <span className="text-text-secondary">Not checked</span>
            ) : state.status === "checking" ? (
              <>
                <Spinner className="h-3 w-3 text-text-muted" />
                <span className="text-text-secondary">Checking</span>
              </>
            ) : (
              <>
                <span className="mono truncate text-[12.5px] font-medium text-text">
                  {state.ip}
                </span>
                <Freshness at={state.checkedAt} />
              </>
            )}
          </>
        )}
      </p>

      {state.status === "ready" ? (
        <IconButton
          label="Copy public IP address"
          size="sm"
          onClick={() => {
            onCopy(state.ip);
            setCopied(true);
            clearTimeout(copyTimer.current);
            copyTimer.current = setTimeout(() => setCopied(false), 1600);
          }}
        >
          {copied ? <Check className="h-3.5 w-3.5 text-online" /> : <Copy className="h-3.5 w-3.5" />}
        </IconButton>
      ) : null}

      {/* The copy confirmation is a live region of its own: the icon swapping
          to a tick is not something a screen reader would otherwise report. */}
      <span role="status" className="sr-only">
        {copied ? "Public IP copied to the clipboard." : ""}
      </span>

      <Button
        size="sm"
        variant={state.status === "error" ? "secondary" : "ghost"}
        aria-label={action.label}
        // Deliberately not disabled while checking: disabling would drop the
        // button out of the focus order mid-interaction. A second press is
        // ignored by the hook rather than starting a second request.
        aria-busy={state.status === "checking" || undefined}
        icon={
          state.status === "checking" ? (
            <Spinner className="h-3.5 w-3.5" />
          ) : state.status === "idle" ? (
            <Globe className="h-3.5 w-3.5" />
          ) : (
            <RotateCw className="h-3.5 w-3.5" />
          )
        }
        onClick={onCheck}
      >
        {action.text}
      </Button>

      {onToggleTechnical ? (
        <button
          type="button"
          onClick={onToggleTechnical}
          aria-expanded={technicalOpen}
          className="rounded text-[11.5px] font-medium text-accent-text underline decoration-dotted underline-offset-2"
        >
          {technicalOpen ? "Hide details" : "Technical details"}
        </button>
      ) : null}
    </div>
  );
}

/**
 * How long ago the address was checked.
 *
 * A public address can change without warning, so a value with no age on it is
 * a value with no way to tell whether it is still true.
 */
function Freshness({ at }: { at: number }) {
  const [, tick] = useState(0);

  useEffect(() => {
    const timer = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(timer);
  }, [at]);

  return (
    <span className="shrink-0 whitespace-nowrap text-text-muted">
      Checked {formatRelative(new Date(at).toISOString())}
    </span>
  );
}
