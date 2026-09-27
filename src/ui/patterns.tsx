// Composite patterns shared by every view.
//
// Each one replaces markup that had been written three or four times with
// small differences — three export popovers, four search fields, two sortable
// header implementations. One implementation is what keeps the views feeling
// like one product.

import { forwardRef, useRef, useState, type ReactNode } from "react";
import { ArrowDown, ArrowUp, ChevronRight, Download, Search, X } from "lucide-react";
import { Button, Field, IconButton } from "./primitives";
import { Popover } from "./Popover";
import type { ExportFormat } from "../types";

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

export interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  /** The field's accessible name, which says what is searched. */
  label: string;
  className?: string;
}

/** The one search field: fixed width, a leading icon, a full-size clear button. */
export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(function SearchField(
  { value, onChange, placeholder, label, className = "w-56" },
  ref,
) {
  return (
    <div className={`relative shrink-0 ${className}`}>
      <Search
        className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-text-muted"
        aria-hidden
      />
      <Field
        ref={ref}
        className="pl-7 pr-7"
        placeholder={placeholder}
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      {value ? (
        <button
          type="button"
          aria-label="Clear the search"
          onClick={() => onChange("")}
          // A full 24px target: the field often sits next to other controls,
          // so a smaller hit area has nowhere to borrow space from.
          className="absolute right-1 top-1/2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded text-text-muted hover:text-text"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      ) : null}
    </div>
  );
});

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

export const EXPORT_FORMATS: Array<[ExportFormat, string]> = [
  ["csv", "CSV spreadsheet"],
  ["json", "JSON"],
  ["xml", "XML"],
];

export interface ExportMenuProps {
  onExport: (format: ExportFormat) => void;
  /** One sentence saying exactly what the export contains. */
  scope?: string;
  disabled?: boolean;
  /** Controlled open state, so a keyboard shortcut can open it. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Icon-only trigger, for a row of per-item actions. */
  compact?: boolean;
  label?: string;
  title?: string;
}

/** The one export control: a button and a menu of formats with its scope stated. */
export function ExportMenu({
  onExport,
  scope,
  disabled = false,
  open: controlledOpen,
  onOpenChange,
  compact = false,
  label = "Export",
  title,
}: ExportMenuProps) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [localOpen, setLocalOpen] = useState(false);
  const open = controlledOpen ?? localOpen;
  const setOpen = (next: boolean) => {
    if (controlledOpen === undefined) setLocalOpen(next);
    onOpenChange?.(next);
  };

  return (
    <div className="relative">
      {compact ? (
        <IconButton
          ref={anchor}
          label={label}
          size="sm"
          disabled={disabled}
          aria-haspopup="dialog"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          <Download className="h-3.5 w-3.5" />
        </IconButton>
      ) : (
        <Button
          ref={anchor}
          size="sm"
          icon={<Download className="h-3.5 w-3.5" />}
          disabled={disabled}
          aria-haspopup="dialog"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          title={title}
        >
          {label}
        </Button>
      )}
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        align="end"
        label="Export format"
        className="w-56 p-1"
      >
        <ul aria-label="Export format">
          {EXPORT_FORMATS.map(([format, name]) => (
            <li key={format}>
              <button
                type="button"
                className="menu-item"
                onClick={() => {
                  setOpen(false);
                  onExport(format);
                }}
              >
                {name}
              </button>
            </li>
          ))}
        </ul>
        {scope ? (
          <p className="border-t border-border px-2.5 pb-1.5 pt-2 text-xs leading-relaxed text-text-muted">
            {scope}
          </p>
        ) : null}
      </Popover>
    </div>
  );
}

// ---------------------------------------------------------------------------
// View header
// ---------------------------------------------------------------------------

/**
 * The line that opens Inventory, Changes and History.
 *
 * It says what the view is for in one sentence. Those sentences are how the
 * three views explain their relationship to each other, so they are not
 * decoration: Inventory is what exists, Changes is what moved, History is what
 * ran.
 */
export function ViewHeader({
  title,
  summary,
  purpose,
  actions,
}: {
  title: string;
  /** The live count, e.g. "21 devices · 19 present". */
  summary?: ReactNode;
  /** One sentence: what this view answers. */
  purpose: string;
  actions?: ReactNode;
}) {
  return (
    <div className="view-header">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
          <h1 className="text-[15px] font-semibold leading-tight text-text">{title}</h1>
          {summary ? (
            <p className="text-xs text-text-secondary" aria-live="polite">
              {summary}
            </p>
          ) : null}
        </div>
        <p className="mt-0.5 text-xs text-text-muted">{purpose}</p>
      </div>
      {actions ? <div className="flex shrink-0 flex-wrap items-center gap-1.5">{actions}</div> : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Table header
// ---------------------------------------------------------------------------

/**
 * A sortable column header. The arrow's space is always reserved so a column
 * never shifts when it becomes the sort, and it appears on hover so a person can
 * see the column is sortable before they click it.
 */
export function SortHeader({
  label,
  active,
  direction,
  align,
  onSort,
  width,
  className = "",
}: {
  label: string;
  active: boolean;
  direction: "asc" | "desc";
  align?: "right";
  onSort: () => void;
  width?: string;
  className?: string;
}) {
  const Arrow = active && direction === "desc" ? ArrowDown : ArrowUp;
  return (
    <th
      scope="col"
      aria-sort={active ? (direction === "asc" ? "ascending" : "descending") : "none"}
      className={`${align === "right" ? "text-right" : ""} ${className}`}
      style={width ? { width } : undefined}
    >
      <button
        type="button"
        onClick={onSort}
        data-active={active}
        className={`sort-btn ${align === "right" ? "flex-row-reverse" : ""}`}
        title={`Sort by ${label.toLowerCase()}`}
      >
        {label}
        <Arrow className="sort-arrow h-3 w-3" aria-hidden />
      </button>
    </th>
  );
}

// ---------------------------------------------------------------------------
// Inspector building blocks
// ---------------------------------------------------------------------------

/** A titled group in an inspector or a settings page. */
export function Section({
  title,
  action,
  children,
  id,
}: {
  title: string;
  action?: ReactNode;
  children: ReactNode;
  id?: string;
}) {
  return (
    <section className="border-b border-border px-4 py-3.5 last:border-b-0" aria-labelledby={id}>
      <div className="mb-2.5 flex min-h-[20px] items-center justify-between gap-3">
        <h3 id={id} className="section-label">
          {title}
        </h3>
        {action}
      </div>
      {children}
    </section>
  );
}

/** A label/value pair inside a `<dl className="facts">`. */
export function Fact({
  label,
  children,
  mono = false,
  title,
}: {
  label: string;
  children: ReactNode;
  mono?: boolean;
  title?: string;
}) {
  return (
    <>
      <dt>{label}</dt>
      <dd className={mono ? "mono" : undefined} title={title}>
        {children}
      </dd>
    </>
  );
}

/** A disclosure for secondary detail: evidence, history, raw values. */
export function Disclosure({
  summary,
  children,
  defaultOpen = false,
  count,
}: {
  summary: string;
  children: ReactNode;
  defaultOpen?: boolean;
  count?: number;
}) {
  return (
    <details className="disclosure" open={defaultOpen}>
      <summary>
        <ChevronRight className="disclosure-chevron h-3.5 w-3.5" aria-hidden />
        {summary}
        {count != null ? <span className="font-normal text-text-muted">{count}</span> : null}
      </summary>
      <div className="mt-1.5 pl-5">{children}</div>
    </details>
  );
}
