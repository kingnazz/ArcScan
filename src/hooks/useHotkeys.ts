import { useEffect } from "react";

export interface HotkeyContext {
  /** Escape closes the drawer first, then stops a scan. */
  onEscape: () => void;
  onFocusFilter: () => void;
  onExport: () => void;
  onRescan: () => void;
  onFocusTarget: () => void;
}

/** True when the event came from somewhere that owns its own key handling. */
function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}

/**
 * Application shortcuts.
 *
 * Only combinations that do not already mean something are claimed. Ctrl/Cmd+R
 * is the exception and is deliberate: in a packaged desktop app there is no page
 * to reload, so rescanning is the useful meaning, and the browser demo is a
 * development aid rather than the product.
 */
export function useHotkeys(ctx: HotkeyContext, enabled = true): void {
  useEffect(() => {
    if (!enabled) return;

    function onKeyDown(event: KeyboardEvent) {
      const mod = event.ctrlKey || event.metaKey;

      if (event.key === "Escape") {
        ctx.onEscape();
        return;
      }

      if (mod && !event.shiftKey && !event.altKey) {
        switch (event.key.toLowerCase()) {
          case "f":
            event.preventDefault();
            ctx.onFocusFilter();
            return;
          case "e":
            event.preventDefault();
            ctx.onExport();
            return;
          case "r":
            event.preventDefault();
            ctx.onRescan();
            return;
          case "l":
            event.preventDefault();
            ctx.onFocusTarget();
            return;
        }
      }

      // A bare "/" focuses the filter, but not while the operator is typing.
      if (event.key === "/" && !mod && !isTextEntry(event.target)) {
        event.preventDefault();
        ctx.onFocusFilter();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [ctx, enabled]);
}
