// The right-side drawer, used for device detail and settings.
//
// It is a sibling of the results table rather than an overlay, so the table stays
// visible and keyboard navigation can move between devices without the drawer
// opening and closing on every step. Below 1,100px it becomes an overlay,
// because two side-by-side panes stop being readable.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { X } from "lucide-react";
import { IconButton } from "./primitives";

const MIN_WIDTH = 320;
const MAX_WIDTH = 620;

export interface DrawerProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  /** Shown under the title in a quieter weight. */
  subtitle?: ReactNode;
  /** Chips or a short fact line under the subtitle. */
  meta?: ReactNode;
  /** An action bar pinned under the header, always reachable without scrolling. */
  toolbar?: ReactNode;
  /** Actions pinned to the bottom, always reachable without scrolling. */
  footer?: ReactNode;
  /** Sections bring their own padding; `flush` removes the body's. */
  flush?: boolean;
  /** A fixed width that overrides the shared, resizable one. */
  fixedWidth?: number;
  /** Rendered as an overlay instead of a pane, for narrow windows. */
  overlay?: boolean;
  width: number;
  onWidthChange: (width: number) => void;
  children: ReactNode;
}

export function Drawer({
  open,
  onClose,
  title,
  subtitle,
  meta,
  toolbar,
  footer,
  flush = false,
  fixedWidth,
  overlay = false,
  width,
  onWidthChange,
  children,
}: DrawerProps) {
  const panelRef = useRef<HTMLElement>(null);
  const [dragging, setDragging] = useState(false);

  // Only the overlay form traps Escape. As a pane the drawer is part of the
  // layout, and the app-level shortcut handler decides what Escape means.
  useEffect(() => {
    if (!open || !overlay) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    }
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [open, overlay, onClose]);

  useEffect(() => {
    if (!dragging) return;
    function onMove(event: MouseEvent) {
      const next = window.innerWidth - event.clientX;
      onWidthChange(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, next)));
    }
    function onUp() {
      setDragging(false);
    }
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    // Without this the pointer flickers between resize and text cursors while
    // dragging across the table underneath.
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    return () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, [dragging, onWidthChange]);

  if (!open) return null;

  const panel = (
    <aside
      ref={panelRef}
      role={overlay ? "dialog" : "complementary"}
      aria-modal={overlay || undefined}
      aria-label={typeof title === "string" ? title : "Details"}
      style={{ width: overlay ? undefined : (fixedWidth ?? width) }}
      className={`flex min-h-0 shrink-0 flex-col border-l border-border bg-surface ${
        overlay
          ? "animate-slide-in-right fixed inset-y-0 right-0 z-40 w-full max-w-[420px] shadow-lg"
          : ""
      }`}
    >
      {/* Resize handle. Keyboard users get the same range via arrow keys. */}
      {!overlay && fixedWidth == null ? (
        <div
          role="separator"
          aria-label="Resize panel"
          aria-orientation="vertical"
          aria-valuenow={width}
          aria-valuemin={MIN_WIDTH}
          aria-valuemax={MAX_WIDTH}
          tabIndex={0}
          onMouseDown={() => setDragging(true)}
          onKeyDown={(event) => {
            const step = event.shiftKey ? 40 : 16;
            if (event.key === "ArrowLeft") {
              event.preventDefault();
              onWidthChange(Math.min(MAX_WIDTH, width + step));
            } else if (event.key === "ArrowRight") {
              event.preventDefault();
              onWidthChange(Math.max(MIN_WIDTH, width - step));
            }
          }}
          className="absolute -left-1 top-0 z-10 h-full w-2 cursor-col-resize hover:bg-accent-subtle"
        />
      ) : null}

      <header className="flex items-start gap-2 border-b border-border bg-surface-raised px-4 pb-3 pt-3">
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] font-semibold leading-tight text-text">{title}</div>
          {subtitle ? (
            <div className="mt-1 truncate text-xs text-text-secondary">{subtitle}</div>
          ) : null}
          {meta ? <div className="mt-2 flex flex-wrap items-center gap-1.5">{meta}</div> : null}
        </div>
        <IconButton label="Close panel" size="sm" className="-mr-1.5 -mt-0.5" onClick={onClose}>
          <X className="h-4 w-4" />
        </IconButton>
      </header>

      {toolbar ? (
        <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border bg-surface-raised px-4 py-2">
          {toolbar}
        </div>
      ) : null}

      <div
        className={`min-h-0 flex-1 overflow-y-auto overscroll-contain ${flush ? "" : "px-4 py-3"}`}
      >
        {children}
      </div>

      {footer ? (
        <footer className="border-t border-border bg-surface-raised px-4 py-2">{footer}</footer>
      ) : null}
    </aside>
  );

  if (!overlay) return <div className="relative flex min-h-0">{panel}</div>;

  return (
    <>
      <div
        className="animate-fade-in fixed inset-0 z-30 bg-black/35"
        onClick={onClose}
        aria-hidden
      />
      {panel}
    </>
  );
}
