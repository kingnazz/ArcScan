// The title bar.
//
// A drag region on both platforms, with the view switcher in the middle and the
// window-level actions on the right. Kept to 38px so it costs almost nothing in
// vertical space, which matters when the results table is the point.

import { DownloadCloud, Moon, Settings, Sun } from "lucide-react";
import { IconButton } from "../ui/primitives";
import { Logo } from "./Logo";
import type { ResolvedTheme } from "../hooks/useTheme";

export type View = "results" | "history" | "changes";

export interface TitleBarProps {
  view: View;
  onViewChange: (view: View) => void;
  changeCount: number;
  hasComparison: boolean;
  theme: ResolvedTheme;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  settingsOpen: boolean;
  onCheckUpdates: () => void;
  updateBusy: boolean;
}

export function TitleBar({
  view,
  onViewChange,
  changeCount,
  hasComparison,
  theme,
  onToggleTheme,
  onOpenSettings,
  settingsOpen,
  onCheckUpdates,
  updateBusy,
}: TitleBarProps) {
  const tabs: Array<{ id: View; label: string; badge?: number; disabled?: boolean }> = [
    { id: "results", label: "Devices" },
    { id: "changes", label: "Changes", badge: changeCount, disabled: !hasComparison },
    { id: "history", label: "History" },
  ];

  return (
    <header
      // Tauri turns this into the native drag region, so the window moves from the
      // bar's empty space exactly as an operator expects on Windows and macOS.
      data-tauri-drag-region
      className="flex h-[38px] shrink-0 items-center gap-2 border-b border-border bg-surface-raised px-2.5"
    >
      <div className="flex shrink-0 items-center gap-2" data-tauri-drag-region>
        <Logo size={18} />
        <span className="text-[13px] font-semibold tracking-tight text-text">ArcScan</span>
      </div>

      <nav className="segmented ml-3" aria-label="View">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            className="segmented-item"
            aria-current={view === tab.id ? "page" : undefined}
            aria-selected={view === tab.id}
            disabled={tab.disabled}
            onClick={() => onViewChange(tab.id)}
            title={
              tab.disabled
                ? "A comparison appears once there is an earlier scan of the same target"
                : undefined
            }
          >
            {tab.label}
            {tab.badge != null && tab.badge > 0 ? (
              <span className="rounded bg-accent-subtle px-1 text-[10.5px] font-semibold text-accent-text">
                {tab.badge}
              </span>
            ) : null}
          </button>
        ))}
      </nav>

      {/* The flexible gap is the drag handle. */}
      <div className="min-w-0 flex-1" data-tauri-drag-region />

      <IconButton label="Check for updates" onClick={onCheckUpdates} size="sm">
        <DownloadCloud className={`h-4 w-4 ${updateBusy ? "animate-pulse" : ""}`} />
      </IconButton>
      <IconButton
        label={theme === "dark" ? "Switch to the light theme" : "Switch to the dark theme"}
        onClick={onToggleTheme}
        size="sm"
      >
        {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
      </IconButton>
      <IconButton label="Settings" onClick={onOpenSettings} active={settingsOpen} size="sm">
        <Settings className="h-4 w-4" />
      </IconButton>
    </header>
  );
}
