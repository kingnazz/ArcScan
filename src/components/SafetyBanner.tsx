import { ShieldAlert } from "lucide-react";

// Persistent warning that ArcScan may only be pointed at networks the operator
// owns or is authorized to assess.
export function SafetyBanner() {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-amber-500/25 bg-amber-500/10 px-3.5 py-2 text-amber-200/90">
      <ShieldAlert className="h-4 w-4 shrink-0 text-amber-400" />
      <p className="text-xs leading-relaxed">
        <span className="font-semibold text-amber-200">Authorized use only.</span> ArcScan performs
        read-only discovery. Only scan networks you own or have explicit written authorization to
        assess. Unauthorized scanning may be illegal.
      </p>
    </div>
  );
}
