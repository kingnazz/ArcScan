// Shared UI primitives.
//
// Deliberately small: each one exists because the same markup was about to be
// repeated, not to build a component library. Everything reads its appearance
// from the tokens in index.css, so nothing here contains a colour.

import { forwardRef, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from "react";
import { Loader2 } from "lucide-react";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md" | "lg";

const VARIANT_CLASS: Record<Variant, string> = {
  primary: "btn-primary",
  secondary: "btn-secondary",
  ghost: "btn-ghost",
  danger: "btn-danger",
};

const SIZE_CLASS: Record<Size, string> = { sm: "btn-sm", md: "", lg: "btn-lg" };

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  /** Shows a spinner and blocks input without changing the button's width. */
  busy?: boolean;
  icon?: ReactNode;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", busy = false, icon, children, className = "", disabled, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={`btn ${VARIANT_CLASS[variant]} ${SIZE_CLASS[size]} ${className}`}
      {...rest}
    >
      {busy ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" aria-hidden /> : icon}
      {children}
    </button>
  );
});

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Required: an icon alone tells a screen reader nothing. */
  label: string;
  size?: "sm" | "md";
  active?: boolean;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, size = "md", active = false, children, className = "", type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      aria-label={label}
      title={label}
      className={`icon-btn ${size === "sm" ? "icon-btn-sm" : ""} ${
        active ? "bg-surface-active text-text" : ""
      } ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
});

// `size` is renamed because the native input attribute of that name means
// something else entirely (a character width).
export interface FieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  scale?: "md" | "lg";
  mono?: boolean;
  /** Marks the field invalid. The message itself is rendered by FieldRow. */
  error?: string | null;
}

export const Field = forwardRef<HTMLInputElement, FieldProps>(function Field(
  { scale = "md", mono = false, error, className = "", ...rest },
  ref,
) {
  return (
    <input
      ref={ref}
      className={`field ${scale === "lg" ? "field-lg" : ""} ${mono ? "font-mono" : ""} ${className}`}
      aria-invalid={error ? true : undefined}
      spellCheck={false}
      {...rest}
    />
  );
});

export function FieldRow({
  label,
  hint,
  error,
  htmlFor,
  children,
}: {
  label: string;
  hint?: ReactNode;
  error?: string | null;
  htmlFor?: string;
  children: ReactNode;
}) {
  return (
    <div>
      <label className="field-label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {error ? (
        <p role="alert" className="mt-1 text-xs text-danger">
          {error}
        </p>
      ) : hint ? (
        <p className="mt-1 text-xs leading-relaxed text-text-muted">{hint}</p>
      ) : null}
    </div>
  );
}

export function Select({
  className = "",
  children,
  ...rest
}: React.SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select className={`field cursor-pointer pr-7 ${className}`} {...rest}>
      {children}
    </select>
  );
}

export type BadgeTone = "neutral" | "accent" | "online" | "new" | "changed" | "missing" | "warning";

/**
 * Written out in full rather than built as `badge-${tone}`: Tailwind only keeps
 * the component classes it can find in the source, and a class assembled at
 * runtime is invisible to it — which, before 1.10, silently rendered every
 * tone as neutral.
 */
const BADGE_CLASS: Record<BadgeTone, string> = {
  neutral: "badge-neutral",
  accent: "badge-accent",
  online: "badge-online",
  new: "badge-new",
  changed: "badge-changed",
  missing: "badge-missing",
  warning: "badge-warning",
};

export function Badge({
  tone = "neutral",
  icon,
  children,
  title,
}: {
  tone?: BadgeTone;
  icon?: ReactNode;
  children: ReactNode;
  title?: string;
}) {
  return (
    <span className={`badge ${BADGE_CLASS[tone]}`} title={title}>
      {icon}
      {children}
    </span>
  );
}

/**
 * A status dot. Never used alone: every caller pairs it with a word or a
 * tooltip, because colour on its own is not a message.
 */
export function StatusDot({ tone, label }: { tone: BadgeTone; label: string }) {
  const color: Record<BadgeTone, string> = {
    neutral: "var(--text-muted)",
    accent: "var(--accent)",
    online: "var(--online)",
    new: "var(--new)",
    changed: "var(--changed)",
    missing: "var(--missing)",
    warning: "var(--warning)",
  };
  return (
    <span
      className="inline-block h-2 w-2 shrink-0 rounded-full"
      style={{ background: color[tone] }}
      role="img"
      aria-label={label}
      title={label}
    />
  );
}

export function SectionHeading({
  children,
  action,
  id,
}: {
  children: ReactNode;
  action?: ReactNode;
  id?: string;
}) {
  return (
    <div className="mb-2 flex min-h-[20px] items-center justify-between gap-3">
      <h3 id={id} className="section-label">
        {children}
      </h3>
      {action}
    </div>
  );
}

/**
 * An empty state. Text and one clear action, no illustration: this is a utility,
 * and a large graphic where data belongs reads as a placeholder.
 */
export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 px-6 py-16 text-center">
      {icon ? <div className="mb-1 text-text-muted">{icon}</div> : null}
      <p className="text-sm font-semibold text-text">{title}</p>
      {description ? (
        <p className="max-w-md text-[13px] leading-relaxed text-text-secondary">{description}</p>
      ) : null}
      {action ? <div className="mt-2 flex flex-wrap justify-center gap-2">{action}</div> : null}
    </div>
  );
}

export function Spinner({ className = "h-4 w-4" }: { className?: string }) {
  return <Loader2 className={`animate-spin ${className}`} aria-hidden />;
}
