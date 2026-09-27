// The Public IP utility.
//
// Lives on the right of the scan status line: it answers "which address does
// the internet see this computer from", which is context for a scan and never
// part of one. It is deliberately quieter than the target field and the Scan
// action beside it.

import { useEffect, useRef, useState } from "react";
import { AlertTriangle, Check, Copy, Globe, RotateCw } from "lucide-react";
import { Button, IconButton, Spinner } from "../ui/primitives";
import { formatRelative } from "../lib/format";
import type { PublicIpState } from "../hooks/usePublicIp";

export interface PublicIpUtilityProps {
  state: PublicIpState;
  onCheck: () => void;
  onCopy: (ip: string) => void;
}

/**
 * The utility and its technical-details panel, as two pieces: the control sits
 * on the status line and the details open on a line of their own beneath it.
 * (A hook rather than a component because the two pieces render apart.)
 */
export function usePublicIpUtility({ state, onCheck, onCopy }: PublicIpUtilityProps) {
  const [showTechnical, setShowTechnical] = useState(false);
  const technical = state.status === "error" ? state.technical : undefined;

  // A failed lookup that is retried should not keep the previous failure's
  // details open underneath the new state.
  useEffect(() => {
    if (state.status !== "error") setShowTechnical(false);
  }, [state.status]);

  return {
    control: (
      <PublicIp
        state={state}
        onCheck={onCheck}
        onCopy={onCopy}
        technicalOpen={showTechnical}
        onToggleTechnical={technical ? () => setShowTechnical((v) => !v) : undefined}
      />
    ),
    details:
      showTechnical && technical ? (
        <pre className="mono mx-3 mb-2 max-h-24 overflow-auto whitespace-pre-wrap rounded border border-border bg-surface-sunken p-2 text-[11px] leading-relaxed text-text-secondary">
          {technical}
        </pre>
      ) : null,
  };
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
        // A real button in the states that ask for a press, quieter once it is
        // only offering to repeat itself. Either way it stays well below the
        // target field and the Scan action in weight.
        variant={state.status === "ready" ? "ghost" : "secondary"}
        aria-label={action.label}
        // Deliberately not disabled while checking: disabling would drop the
        // button out of the focus order mid-interaction. A second press is
        // ignored by the hook rather than starting a second request.
        aria-busy={state.status === "checking" || undefined}
        // No globe here: the status beside it already carries one, and two of
        // the same mark in one control group is noise.
        icon={
          state.status === "checking" ? (
            <Spinner className="h-3.5 w-3.5" />
          ) : state.status === "idle" ? undefined : (
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
