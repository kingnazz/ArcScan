import { HelpCircle, MonitorSmartphone, Network, Server, Sparkles } from "lucide-react";
import type { DashboardStats } from "../types";

interface StatCardProps {
  label: string;
  value: number | string;
  icon: React.ReactNode;
  accent?: string;
  hint?: string;
}

function StatCard({ label, value, icon, accent = "text-brand-300", hint }: StatCardProps) {
  return (
    <div className="panel group relative overflow-hidden p-4">
      <div className="flex items-start justify-between">
        <div>
          <div className="text-xs font-medium uppercase tracking-wide text-slate-500">{label}</div>
          <div className="mt-1 text-2xl font-semibold tabular-nums text-slate-100">{value}</div>
          {hint && <div className="mt-1 text-[11px] text-slate-500">{hint}</div>}
        </div>
        <div className={`rounded-lg bg-white/5 p-2 ${accent}`}>{icon}</div>
      </div>
      <div className="pointer-events-none absolute -right-6 -bottom-6 h-20 w-20 rounded-full bg-brand-500/5 blur-xl transition-opacity group-hover:opacity-100" />
    </div>
  );
}

export function Dashboard({ stats }: { stats: DashboardStats }) {
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
      <StatCard
        label="Devices found"
        value={stats.total}
        icon={<Network className="h-5 w-5" />}
      />
      <StatCard
        label="Unknown"
        value={stats.unknown}
        icon={<HelpCircle className="h-5 w-5" />}
        accent="text-slate-300"
        hint="No vendor identified"
      />
      <StatCard
        label="Open RDP"
        value={stats.openRdp}
        icon={<MonitorSmartphone className="h-5 w-5" />}
        accent={stats.openRdp > 0 ? "text-amber-300" : "text-slate-300"}
        hint="Port 3389"
      />
      <StatCard
        label="Open SMB"
        value={stats.openSmb}
        icon={<Server className="h-5 w-5" />}
        accent={stats.openSmb > 0 ? "text-amber-300" : "text-slate-300"}
        hint="Port 445"
      />
      <StatCard
        label="New devices"
        value={stats.newDevices}
        icon={<Sparkles className="h-5 w-5" />}
        accent={stats.newDevices > 0 ? "text-brand-300" : "text-slate-300"}
        hint="Since last scan"
      />
    </div>
  );
}
