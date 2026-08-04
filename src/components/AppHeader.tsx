// The application header.
//
// The first in-app row: the view switcher on the left, the window-level actions
// on the right, and nothing in between but the window's drag surface.
//
// It deliberately carries no ArcScan icon or wordmark. The window is already
// titled "ArcScan" by the operating system (`decorations: true` in
// tauri.conf.json), so a second mark here rendered the name and the icon twice
// on every launch. Removing it also gives the view switcher the start of the
// row, which is where a switcher belongs.

import { DownloadCloud, Moon, Settings, Sun } from "lucide-react";
import { IconButton } from "../ui/primitives";
import type { ResolvedTheme } from "../hooks/useTheme";

export type View = "results" | "inventory" | "changes" | "history";

export interface AppHeaderProps {
  view: View;
  onViewChange: (view: View) => void;
  /** Devices in the persistent inventory. */
  inventoryCount: number;
  /** Unreviewed entries in the Changes inbox. */
  unreviewedChanges: number;
  theme: ResolvedTheme;
  onToggleTheme: () => void;
  onOpenSettings: () => void;
  settingsOpen: boolean;
  onCheckUpdates: () => void;
  updateBusy: boolean;
}

export function AppHeader({
  view,
  onViewChange,
  inventoryCount,
  unreviewedChanges,
  theme,
  onToggleTheme,
  onOpenSettings,
  settingsOpen,
  onCheckUpdates,
  updateBusy,
}: AppHeaderProps) {
  // Four views, none of them ever disabled: Inventory and Changes explain their
  // own empty states, which is more useful than a tab that cannot be clicked.
  //
  // `countNoun` is what the badge means, read out for anyone who cannot see
  // that it sits next to the Inventory tab rather than the Changes one.
  const tabs: Array<{ id: View; label: string; badge?: number; countNoun?: string }> = [
    { id: "results", label: "Scan" },
    { id: "inventory", label: "Inventory", badge: inventoryCount, countNoun: "devices" },
    { id: "changes", label: "Changes", badge: unreviewedChanges, countNoun: "unreviewed" },
    { id: "history", label: "History" },
  ];

  return (
    <header
      // Tauri turns this into the native drag region, so the window moves from the
      // header's empty space exactly as an operator expects on Windows and macOS.
      data-tauri-drag-region
      className="flex h-[38px] shrink-0 items-center gap-2 border-b border-border bg-surface-raised px-2.5"
    >
      <nav className="segmented" aria-label="View">
        {tabs.map((tab) => {
          const showBadge = tab.badge != null && tab.badge > 0;
          return (
            <button
              key={tab.id}
              type="button"
              className="segmented-item"
              // aria-current is the right signal inside a nav, and aria-selected
              // is not a valid attribute on a plain button.
              aria-current={view === tab.id ? "page" : undefined}
              onClick={() => onViewChange(tab.id)}
            >
              {tab.label}
              {showBadge ? (
                <>
                  <span className="nav-badge" aria-hidden>
                    {tab.badge! > 999 ? "999+" : tab.badge}
                  </span>
                  <span className="sr-only">
                    , {tab.badge} {tab.countNoun}
                  </span>
                </>
              ) : null}
            </button>
          );
        })}
      </nav>

      {/* The flexible gap is the drag handle. */}
      <div className="min-w-0 flex-1" data-tauri-drag-region />

      <div className="flex shrink-0 items-center gap-0.5">
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
      </div>
    </header>
  );
}
