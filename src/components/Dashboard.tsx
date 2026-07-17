import { HelpCircle, MonitorSmartphone, Network, Server, Sparkles } from "lucide-react";
import type { DashboardStats } from "../types";

interface StatCardProps {
  label: string;
  value: number | string;
  icon: React.ReactNode;
  accent?: string;
  hint?: string;
}

function StatCard({ label, value, icon, accent = "text-brand-600", hint }: StatCardProps) {
  return (
    <div className="panel group relative overflow-hidden p-4">
      <div className="flex items-start justify-between">
        <div>
          <div className="text-xs font-medium uppercase tracking-wide text-faint">{label}</div>
          <div className="mt-1 text-2xl font-semibold tabular-nums text-fg">{value}</div>
          {hint && <div className="mt-1 text-[11px] text-faint">{hint}</div>}
        </div>
        <div className={`rounded-lg bg-brand-500/10 p-2 ${accent}`}>{icon}</div>
      </div>
      <div className="pointer-events-none absolute -right-6 -bottom-6 h-20 w-20 rounded-full bg-brand-500/5 blur-xl" />
    </div>
  );
}

export function Dashboard({ stats }: { stats: DashboardStats }) {
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
      <StatCard label="Devices found" value={stats.total} icon={<Network className="h-5 w-5" />} />
      <StatCard
        label="Unknown"
        value={stats.unknown}
        icon={<HelpCircle className="h-5 w-5" />}
        accent="text-muted"
        hint="No vendor identified"
      />
      <StatCard
        label="Open RDP"
        value={stats.openRdp}
        icon={<MonitorSmartphone className="h-5 w-5" />}
        accent={stats.openRdp > 0 ? "text-amber-500" : "text-muted"}
        hint="Port 3389"
      />
      <StatCard
        label="Open SMB"
        value={stats.openSmb}
        icon={<Server className="h-5 w-5" />}
        accent={stats.openSmb > 0 ? "text-amber-500" : "text-muted"}
        hint="Port 445"
      />
      <StatCard
        label="New devices"
        value={stats.newDevices}
        icon={<Sparkles className="h-5 w-5" />}
        accent={stats.newDevices > 0 ? "text-brand-600" : "text-muted"}
        hint="Since last scan"
      />
    </div>
  );
}
