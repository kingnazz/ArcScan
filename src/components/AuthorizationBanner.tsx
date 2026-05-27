import { ShieldCheck, ShieldAlert } from "lucide-react";

interface AuthorizationBannerProps {
  authorized: boolean;
  onChange: (value: boolean) => void;
}

export function AuthorizationBanner({ authorized, onChange }: AuthorizationBannerProps) {
  return (
    <div
      className={`panel px-4 py-3 flex items-start gap-3 border-l-2 ${
        authorized ? "border-l-ok" : "border-l-warn"
      }`}
    >
      <div className={authorized ? "text-ok" : "text-warn"}>
        {authorized ? (
          <ShieldCheck className="h-5 w-5 mt-0.5" />
        ) : (
          <ShieldAlert className="h-5 w-5 mt-0.5" />
        )}
      </div>
      <div className="flex-1 min-w-0">
        <p className="text-sm text-slate-200 font-medium">Authorized use only</p>
        <p className="text-xs text-slate-400 mt-0.5 leading-relaxed">
          Only scan networks you own or have explicit written authorization to assess.
          Unauthorized network scanning may be illegal. ArcScan performs read-only host
          discovery — it never attempts credential, brute-force, or exploitation activity.
        </p>
        <label className="mt-2 inline-flex items-center gap-2 cursor-pointer select-none">
          <input
            type="checkbox"
            checked={authorized}
            onChange={(e) => onChange(e.target.checked)}
            className="h-4 w-4 rounded border-base-600 bg-base-850 text-accent focus:ring-accent/40"
          />
          <span className="text-xs text-slate-300">
            I am authorized to scan the target network.
          </span>
        </label>
      </div>
    </div>
  );
}
