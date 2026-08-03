// Scan comparison.
//
// Three separate groups rather than one merged list, because "arrived", "gone"
// and "different" call for different responses. Every group carries an icon and a
// word as well as a colour, so the comparison is readable without relying on red
// and green.

import { ArrowRightLeft, MinusCircle, PlusCircle, RotateCcw } from "lucide-react";
import { Badge, Button, EmptyState, SectionHeading } from "../ui/primitives";
import { ChangeList } from "./DeviceDrawer";
import { formatDateTime, formatRelative } from "../lib/format";
import type { DeviceDiff, ScanComparison } from "../types";

export interface ComparisonPanelProps {
  comparison: ScanComparison;
  currentLabel: string;
  /** True when the shown scan was stopped early, so changes cannot exist. */
  partial?: boolean;
  /** Return to the device table. */
  onBack?: () => void;
}

export function ComparisonPanel({ comparison, currentLabel, partial, onBack }: ComparisonPanelProps) {
  const total = comparison.added.length + comparison.removed.length + comparison.changed.length;

  if (comparison.baseline_scan_id == null) {
    return (
      <EmptyState
        title={
          partial ? "Changes unavailable for this partial scan" : "Nothing to compare yet"
        }
        description={
          comparison.reason ??
          "A comparison needs an earlier completed scan with the same target and coverage. Run this scan again later and the differences will appear here."
        }
        action={onBack ? <Button onClick={onBack}>Back to devices</Button> : undefined}
      />
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <header className="border-b border-border bg-surface-raised px-4 py-3">
        <h2 className="text-sm font-semibold text-text">
          {total === 0
            ? "No changes since the previous scan"
            : `${total} ${total === 1 ? "change" : "changes"} since the previous scan`}
        </h2>
        <p className="mt-1 text-xs leading-relaxed text-text-secondary">
          Comparing <span className="mono text-text">{currentLabel}</span> with the scan from{" "}
          {comparison.baseline_created_at ? (
            <span title={comparison.baseline_created_at}>
              {formatDateTime(comparison.baseline_created_at)}
            </span>
          ) : (
            "the previous run"
          )}
          .
        </p>
      </header>

      {total === 0 ? (
        <EmptyState
          title="Everything matched"
          description="The same devices answered, at the same addresses, with the same open services."
        />
      ) : (
        <div className="space-y-5 px-4 py-4">
          <Group
            title="Added devices"
            icon={<PlusCircle className="h-3.5 w-3.5 text-new" aria-hidden />}
            entries={comparison.added}
            emptyLabel="No devices arrived."
          />
          <Group
            title="Missing devices"
            icon={<MinusCircle className="h-3.5 w-3.5 text-missing" aria-hidden />}
            entries={comparison.removed}
            emptyLabel="No devices went missing."
          />
          <Group
            title="Changed devices"
            icon={<ArrowRightLeft className="h-3.5 w-3.5 text-changed" aria-hidden />}
            entries={comparison.changed}
            emptyLabel="No devices changed."
          />
        </div>
      )}
    </div>
  );
}

function Group({
  title,
  icon,
  entries,
  emptyLabel,
}: {
  title: string;
  icon: React.ReactNode;
  entries: DeviceDiff[];
  emptyLabel: string;
}) {
  return (
    <section>
      <SectionHeading>
        <span className="inline-flex items-center gap-1.5">
          {icon}
          {title}
          <span className="text-text-muted">({entries.length})</span>
        </span>
      </SectionHeading>
      {entries.length === 0 ? (
        <p className="text-[13px] text-text-muted">{emptyLabel}</p>
      ) : (
        <ul className="space-y-2">
          {entries.map((entry) => (
            <li
              key={`${entry.kind}-${entry.ip}-${entry.device_id ?? "unknown"}`}
              className="surface-panel px-3 py-2.5"
            >
              <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
                <span className="text-[13px] font-medium text-text">{entry.name}</span>
                <span className="mono text-xs text-text-secondary">{entry.ip}</span>
                <KindBadge kind={entry.kind} />
              </div>

              <p className="mt-0.5 text-xs text-text-muted">
                {entry.vendor ? <>{entry.vendor} · </> : null}
                {entry.mac ? <span className="mono">{entry.mac}</span> : "No MAC address"}
                {entry.kind === "missing" && entry.last_seen ? (
                  <> · last seen {formatRelative(entry.last_seen)}</>
                ) : null}
              </p>

              {entry.fields.length > 0 ? (
                <div className="mt-2 border-t border-border pt-2">
                  <ChangeList changes={entry.fields} />
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function KindBadge({ kind }: { kind: DeviceDiff["kind"] }) {
  switch (kind) {
    case "new":
      return <Badge tone="new">New device</Badge>;
    case "returned":
      return (
        <Badge tone="accent" icon={<RotateCcw className="h-2.5 w-2.5" aria-hidden />}>
          Returned
        </Badge>
      );
    case "missing":
      return <Badge tone="missing">Missing</Badge>;
    case "changed":
      return <Badge tone="changed">Changed</Badge>;
  }
}
