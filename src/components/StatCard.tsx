import type { LucideIcon } from "lucide-react";

interface StatCardProps {
  label: string;
  value: number | string;
  icon: LucideIcon;
  tone?: "default" | "accent" | "warn" | "danger" | "ok";
  hint?: string;
}

const TONES: Record<NonNullable<StatCardProps["tone"]>, string> = {
  default: "text-slate-200 bg-base-700/40",
  accent: "text-accent-soft bg-accent/10",
  warn: "text-warn bg-warn/10",
  danger: "text-danger bg-danger/10",
  ok: "text-ok bg-ok/10",
};

export function StatCard({ label, value, icon: Icon, tone = "default", hint }: StatCardProps) {
  return (
    <div className="panel p-4 flex items-start gap-4 animate-fade-in">
      <div className={`shrink-0 rounded-lg p-2.5 ${TONES[tone]}`}>
        <Icon className="h-5 w-5" />
      </div>
      <div className="min-w-0">
        <div className="text-2xl font-semibold tabular-nums leading-tight text-white">
          {value}
        </div>
        <div className="text-xs font-medium uppercase tracking-wide text-slate-400 mt-0.5">
          {label}
        </div>
        {hint && <div className="text-xs text-slate-500 mt-1 truncate">{hint}</div>}
      </div>
    </div>
  );
}
