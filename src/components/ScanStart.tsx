// The results area before a scan has run.
//
// Its job is to get the operator scanning, so it leads with the detected
// network and one action, and says what will happen when they press it. It sits
// where the table will be and is aligned to it rather than floating in the
// middle of the window, so the first scan fills the same space this occupies.
// No illustration: a large graphic where the results table belongs would read
// as a placeholder.

import { History, Network, Play, ShieldCheck } from "lucide-react";
import { Button } from "../ui/primitives";
import { DEPTHS, PROFILES, type ProfileId } from "../lib/profiles";
import type { LocalNetwork, ScanDepth } from "../types";

export interface ScanStartProps {
  localNetworks: LocalNetwork[];
  /** Profile recommendation for a target, evaluated per displayed network. */
  recommendFor: (target: string) => ProfileId;
  /** The depth the scan bar is set to, so the description matches the scan. */
  depth: ScanDepth;
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
  depth,
  recents,
  showGuidance,
  onScanNetwork,
  onPickTarget,
  onDismissGuidance,
}: ScanStartProps) {
  const primary = localNetworks[0];
  const recommended = primary ? recommendFor(primary.cidr) : "quick-lan";
  const others = localNetworks.slice(1);
  const recentTargets = recents.filter((r) => r !== primary?.cidr);

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <div className="max-w-3xl px-4 py-6">
        <h2 className="text-[15px] font-semibold text-text">Scan a network</h2>
        <p className="mt-1 text-[13px] leading-relaxed text-text-secondary">
          A scan finds the hosts that answer, works out what each device is, then resolves names.
          Devices appear here as they answer, and Stop keeps whatever has been found.
        </p>

        {primary ? (
          <div className="surface-panel mt-4 flex flex-wrap items-center gap-x-6 gap-y-3 px-4 py-3.5">
            <div className="min-w-0 flex-1">
              <p className="flex items-center gap-1.5 text-xs text-text-muted">
                <Network className="h-3.5 w-3.5" aria-hidden />
                This computer's network
              </p>
              <p className="mono mt-1 text-base font-semibold text-text">{primary.cidr}</p>
              <p className="mt-0.5 text-xs text-text-secondary">
                {primary.interface} · this computer is{" "}
                <span className="mono text-text">{primary.ip}</span>
              </p>
              <p className="mt-2 text-xs text-text-secondary">
                <span className="font-medium text-text">{PROFILES[recommended].name}</span> profile,{" "}
                <span className="font-medium text-text">{DEPTHS[depth].name}</span> depth ·{" "}
                {PROFILES[recommended].summary}
              </p>
            </div>
            <Button
              variant="primary"
              size="lg"
              icon={<Play className="h-3.5 w-3.5 fill-current" />}
              title={`Scan ${primary.cidr} with the ${PROFILES[recommended].name} profile`}
              onClick={() => onScanNetwork(primary.cidr, recommended)}
            >
              Scan {primary.cidr}
            </Button>
          </div>
        ) : (
          <p className="surface-panel mt-4 px-4 py-3.5 text-[13px] leading-relaxed text-text-secondary">
            ArcScan could not detect a local network on this computer. Enter a target in the field
            above: a single address, a dashed range such as <span className="mono">10.0.0.1-50</span>,
            or a CIDR block such as <span className="mono">192.168.1.0/24</span>.
          </p>
        )}

        {others.length > 0 || recentTargets.length > 0 ? (
          <div className="mt-4 grid gap-4 sm:grid-cols-2">
            {others.length > 0 ? (
              <section>
                <h3 className="section-label">Other networks on this computer</h3>
                <ul className="mt-1.5 space-y-0.5">
                  {others.map((network) => (
                    <li key={network.cidr}>
                      <button
                        type="button"
                        onClick={() => onPickTarget(network.cidr)}
                        className="menu-item -ml-2.5"
                        title={`Put ${network.cidr} in the target field`}
                      >
                        <span className="mono text-accent-text">{network.cidr}</span>
                        <span className="text-xs text-text-muted">{network.interface}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            ) : null}
            {recentTargets.length > 0 ? (
              <section>
                <h3 className="section-label">Recent targets</h3>
                <ul className="mt-1.5 space-y-0.5">
                  {recentTargets.slice(0, 6).map((target) => (
                    <li key={target}>
                      <button
                        type="button"
                        onClick={() => onPickTarget(target)}
                        className="menu-item -ml-2.5"
                        title={`Put ${target} in the target field`}
                      >
                        <History className="h-3.5 w-3.5 shrink-0 text-text-muted" aria-hidden />
                        <span className="mono text-text-secondary">{target}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            ) : null}
          </div>
        ) : null}

        {showGuidance ? (
          <section
            aria-labelledby="scan-guidance"
            className="mt-6 flex gap-3 border-t border-border pt-4 text-xs leading-relaxed text-text-secondary"
          >
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-accent-text" aria-hidden />
            <div className="min-w-0">
              <h3 id="scan-guidance" className="text-[13px] font-medium text-text">
                What a scan does
              </h3>
              <p className="mt-1">
                ArcScan sends ICMP echo requests and attempts TCP connections to the addresses you
                give it, then reads this computer's own ARP table. It is read-only discovery: it never
                tries a password, never sends an exploit, and never uploads your results. No
                administrator or root privileges are needed.
              </p>
              <p className="mt-1">Only scan networks you own or are authorised to inspect.</p>
              <button type="button" className="link-btn mt-1.5" onClick={onDismissGuidance}>
                Got it, hide this
              </button>
            </div>
          </section>
        ) : null}
      </div>
    </div>
  );
}
