// The screen before a scan has run.
//
// Its job is to get the operator scanning, so it leads with the detected network
// and one action. No illustration: a large graphic where the results table belongs
// would read as a placeholder.

import { Play, Shield } from "lucide-react";
import { Button } from "../ui/primitives";
import { PROFILES, type ProfileId } from "../lib/profiles";
import type { LocalNetwork } from "../types";

export interface ScanStartProps {
  localNetworks: LocalNetwork[];
  /** Profile recommendation for a target, evaluated per displayed network. */
  recommendFor: (target: string) => ProfileId;
  recents: string[];
  showGuidance: boolean;
  /**
   * Start a scan of `cidr` with `profile` — always the recommendation shown
   * beside the button, so the action can never disagree with its description.
   */
  onScanNetwork: (cidr: string, profile: ProfileId) => void;
  onPickTarget: (target: string) => void;
  onDismissGuidance: () => void;
}

export function ScanStart({
  localNetworks,
  recommendFor,
  recents,
  showGuidance,
  onScanNetwork,
  onPickTarget,
  onDismissGuidance,
}: ScanStartProps) {
  const primary = localNetworks[0];
  const recommended = primary ? recommendFor(primary.cidr) : "quick-lan";

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <div className="mx-auto max-w-xl px-6 py-10">
        <h2 className="text-base font-semibold text-text">Scan a network</h2>

        {primary ? (
          <div className="surface-panel mt-3 p-4">
            <p className="text-xs font-semibold uppercase tracking-wide text-text-muted">
              Detected network
            </p>
            <p className="mono mt-1 text-lg font-semibold text-text">{primary.cidr}</p>
            <p className="mt-1 text-[13px] text-text-secondary">
              {primary.interface} · this computer is{" "}
              <span className="mono text-text">{primary.ip}</span>
            </p>
            <p className="mt-2.5 text-[13px] leading-relaxed text-text-secondary">
              Recommended profile:{" "}
              <span className="font-medium text-text">{PROFILES[recommended].name}</span>.{" "}
              {PROFILES[recommended].summary}.
            </p>
            <Button
              variant="primary"
              className="mt-3"
              icon={<Play className="h-3.5 w-3.5" />}
              title={`Scan ${primary.cidr} with the ${PROFILES[recommended].name} profile`}
              onClick={() => onScanNetwork(primary.cidr, recommended)}
            >
              Scan {primary.cidr}
            </Button>
          </div>
        ) : (
          <p className="mt-3 text-[13px] leading-relaxed text-text-secondary">
            ArcScan could not detect a local network on this computer. Enter a target in the field
            above: a single address, a dashed range such as{" "}
            <span className="mono">10.0.0.1-50</span>, or a CIDR block such as{" "}
            <span className="mono">192.168.1.0/24</span>.
          </p>
        )}

        {localNetworks.length > 1 ? (
          <section className="mt-5">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-text-muted">
              Other networks on this computer
            </h3>
            <ul className="mt-1.5 space-y-1">
              {localNetworks.slice(1).map((network) => (
                <li key={network.cidr}>
                  <button
                    type="button"
                    onClick={() => onPickTarget(network.cidr)}
                    className="mono rounded px-1.5 py-0.5 text-[13px] text-accent-text hover:bg-surface-hover"
                  >
                    {network.cidr}
                  </button>
                  <span className="ml-1.5 text-xs text-text-muted">{network.interface}</span>
                </li>
              ))}
            </ul>
          </section>
        ) : null}

        {recents.length > 0 ? (
          <section className="mt-5">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-text-muted">
              Recent targets
            </h3>
            <ul className="mt-1.5 flex flex-wrap gap-1.5">
              {recents.map((target) => (
                <li key={target}>
                  <button
                    type="button"
                    onClick={() => onPickTarget(target)}
                    className="mono rounded-md border border-border bg-surface px-2 py-1 text-xs text-text-secondary transition-colors duration-fast hover:border-border-strong hover:text-text"
                  >
                    {target}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ) : null}

        {showGuidance ? (
          <section className="mt-6 rounded-lg border border-border bg-surface-sunken p-3.5">
            <h3 className="flex items-center gap-1.5 text-[13px] font-semibold text-text">
              <Shield className="h-3.5 w-3.5 text-accent-text" aria-hidden />
              What a scan does
            </h3>
            <ul className="mt-2 space-y-1.5 text-[13px] leading-relaxed text-text-secondary">
              <li>
                ArcScan sends ICMP echo requests and attempts TCP connections to the addresses you
                give it, then reads your computer's own ARP table. That is all: it is read-only
                discovery.
              </li>
              <li>
                It never tries a password, never sends an exploit, and never uploads your results.
                Everything stays in a database on this computer.
              </li>
              <li>
                No administrator or root privileges are needed, because it uses ordinary operating
                system networking.
              </li>
              <li>Only scan networks you own or are authorised to inspect.</li>
            </ul>
            <Button size="sm" variant="ghost" className="mt-2.5" onClick={onDismissGuidance}>
              Got it, hide this
            </Button>
          </section>
        ) : null}
      </div>
    </div>
  );
}
