// Popover and Tooltip.
//
// Both keep themselves inside the window, because the app is 940px wide at its
// narrowest and a menu that opens off the right edge is unusable. Both close on
// Escape and on a click outside, and Escape is stopped from bubbling so it
// dismisses the popover rather than also cancelling a running scan.

import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

export interface PopoverProps {
  open: boolean;
  onClose: () => void;
  /** The control the popover belongs to, used for positioning and focus return. */
  anchor: React.RefObject<HTMLElement>;
  align?: "start" | "end";
  /** Announced as the popover's name. */
  label: string;
  className?: string;
  children: ReactNode;
}

export function Popover({
  open,
  onClose,
  anchor,
  align = "end",
  label,
  className = "",
  children,
}: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [offset, setOffset] = useState(0);

  // Nudge the panel back inside the window when the anchor sits near an edge.
  useLayoutEffect(() => {
    if (!open || !ref.current) return;
    setOffset(0);
    const rect = ref.current.getBoundingClientRect();
    const margin = 8;
    if (rect.right > window.innerWidth - margin) {
      setOffset(-(rect.right - window.innerWidth + margin));
    } else if (rect.left < margin) {
      setOffset(margin - rect.left);
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;

    function onPointerDown(event: MouseEvent) {
      const target = event.target as Node;
      if (ref.current?.contains(target)) return;
      if (anchor.current?.contains(target)) return;
      onClose();
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
        anchor.current?.focus();
      }
    }

    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open, onClose, anchor]);

  if (!open) return null;

  return (
    <div
      ref={ref}
      role="dialog"
      aria-label={label}
      style={{ transform: `translateX(${offset}px)` }}
      className={`popover animate-slide-up absolute mt-1.5 ${
        align === "end" ? "right-0" : "left-0"
      } ${className}`}
    >
      {children}
    </div>
  );
}

/**
 * A tooltip for things a title attribute cannot express well: multi-line change
 * summaries, and hints that must be reachable by keyboard.
 *
 * It is bound with aria-describedby rather than aria-label, so it supplements the
 * control's name instead of replacing it.
 */
export function Tooltip({
  content,
  children,
  side = "top",
}: {
  content: ReactNode;
  children: ReactNode;
  side?: "top" | "bottom";
}) {
  const [open, setOpen] = useState(false);
  const id = useId();

  return (
    <span
      className="relative inline-flex"
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)}
      onBlur={() => setOpen(false)}
    >
      <span aria-describedby={open ? id : undefined}>{children}</span>
      {open ? (
        <span
          role="tooltip"
          id={id}
          className={`popover animate-fade-in pointer-events-none absolute left-1/2 z-50 w-max max-w-xs -translate-x-1/2 px-2.5 py-1.5 text-xs leading-relaxed text-text ${
            side === "top" ? "bottom-full mb-1.5" : "top-full mt-1.5"
          }`}
        >
          {content}
        </span>
      ) : null}
    </span>
  );
}
