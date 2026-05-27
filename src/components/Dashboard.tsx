import { useMemo } from "react";
import { Boxes, HelpCircle, MonitorSmartphone, Network, Sparkles } from "lucide-react";
import type { Host } from "../types";
import { StatCard } from "./StatCard";

interface DashboardProps {
  hosts: Host[];
}

export function Dashboard({ hosts }: DashboardProps) {
  const stats = useMemo(() => {
    const live = hosts.filter((h) => h.status === "up");
    const unknown = live.filter((h) => !h.hostname && !h.vendor).length;
    const rdp = live.filter((h) => h.openPorts.some((p) => p.port === 3389)).length;
    const smb = live.filter((h) => h.openPorts.some((p) => p.port === 445)).length;
    const isNew = live.filter((h) => h.isNew).length;
    return { total: live.length, unknown, rdp, smb, isNew };
  }, [hosts]);

  return (
    <div className="grid grid-cols-2 lg:grid-cols-5 gap-3">
      <StatCard label="Devices Found" value={stats.total} icon={Network} tone="accent" />
      <StatCard label="Unknown Devices" value={stats.unknown} icon={HelpCircle} tone="warn" />
      <StatCard label="Open RDP" value={stats.rdp} icon={MonitorSmartphone} tone="danger" />
      <StatCard label="Open SMB" value={stats.smb} icon={Boxes} tone="danger" />
      <StatCard label="New Since Last Scan" value={stats.isNew} icon={Sparkles} tone="ok" />
    </div>
  );
}
