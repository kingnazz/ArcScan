// Toasts and inline error presentation.
//
// One notification system for the whole app. Confirmations are brief and
// self-dismissing; failures stay until dismissed, explain what went wrong in
// plain language, and keep the raw error under an expandable section rather than
// putting a Rust or JavaScript message in front of the operator as the headline.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { AlertTriangle, Check, Info, Undo2, X } from "lucide-react";
import { Button, IconButton } from "./primitives";

export type ToastTone = "success" | "error" | "info";

export interface ToastOptions {
  tone?: ToastTone;
  /** What the operator can do next. Shown as a second line. */
  detail?: string;
  /** The underlying error, hidden behind "Technical details". */
  technical?: string;
  /** Offers an Undo button for as long as the toast is visible. */
  onUndo?: () => void;
  /** Milliseconds before auto-dismiss. Errors never auto-dismiss. */
  duration?: number;
}

interface Toast extends ToastOptions {
  id: number;
  message: string;
  tone: ToastTone;
}

interface ToastApi {
  /** A brief confirmation, e.g. "IP address copied". */
  success: (message: string, options?: ToastOptions) => void;
  /**
   * A failure. `message` says what failed in the operator's terms; put the raw
   * error in `technical` so it is available without being the headline.
   */
  error: (message: string, options?: ToastOptions) => void;
  info: (message: string, options?: ToastOptions) => void;
  dismiss: (id: number) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

const DEFAULT_DURATION = 4_000;
/** Enough to notice and act, since undoing is the point of the toast. */
const UNDO_DURATION = 8_000;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(1);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());

  const dismiss = useCallback((id: number) => {
    setToasts((list) => list.filter((t) => t.id !== id));
    const timer = timers.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timers.current.delete(id);
    }
  }, []);

  const push = useCallback(
    (message: string, tone: ToastTone, options: ToastOptions = {}) => {
      const id = nextId.current++;
      // Errors stay until dismissed: an operator who looked away must not lose
      // the only explanation of why something did not happen.
      const duration =
        options.duration ??
        (tone === "error" ? 0 : options.onUndo ? UNDO_DURATION : DEFAULT_DURATION);

      setToasts((list) => {
        // Four is enough to see a burst without burying the results table.
        const next = [...list, { ...options, id, message, tone }];
        return next.slice(-4);
      });

      if (duration > 0) {
        timers.current.set(
          id,
          setTimeout(() => dismiss(id), duration),
        );
      }
    },
    [dismiss],
  );

  useEffect(
    () => () => {
      for (const timer of timers.current.values()) clearTimeout(timer);
      timers.current.clear();
    },
    [],
  );

  const api = useMemo<ToastApi>(
    () => ({
      success: (message, options) => push(message, "success", options),
      error: (message, options) => push(message, "error", options),
      info: (message, options) => push(message, "info", options),
      dismiss,
    }),
    [push, dismiss],
  );

  return (
    <ToastContext.Provider value={api}>
      {children}
      <ToastViewport toasts={toasts} onDismiss={dismiss} />
    </ToastContext.Provider>
  );
}

export function useToast(): ToastApi {
  const api = useContext(ToastContext);
  if (!api) throw new Error("useToast must be used inside a ToastProvider.");
  return api;
}

function ToastViewport({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  return (
    <div
      // Errors are assertive because they report something that did not happen;
      // confirmations are polite so they never interrupt a screen reader.
      className="pointer-events-none fixed bottom-10 right-3 z-[60] flex w-[min(24rem,calc(100vw-1.5rem))] flex-col gap-2"
    >
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

const TONE_ICON: Record<ToastTone, ReactNode> = {
  success: <Check className="h-4 w-4 shrink-0 text-online" aria-hidden />,
  error: <AlertTriangle className="h-4 w-4 shrink-0 text-danger" aria-hidden />,
  info: <Info className="h-4 w-4 shrink-0 text-accent-text" aria-hidden />,
};

const TONE_LABEL: Record<ToastTone, string> = {
  success: "Done",
  error: "Problem",
  info: "Note",
};

function ToastCard({ toast, onDismiss }: { toast: Toast; onDismiss: (id: number) => void }) {
  const [showTechnical, setShowTechnical] = useState(false);

  return (
    <div
      role={toast.tone === "error" ? "alert" : "status"}
      aria-live={toast.tone === "error" ? "assertive" : "polite"}
      className="popover animate-slide-up pointer-events-auto flex gap-2.5 px-3 py-2.5"
    >
      {TONE_ICON[toast.tone]}
      <div className="min-w-0 flex-1">
        <span className="sr-only">{TONE_LABEL[toast.tone]}: </span>
        <p className="text-[13px] font-medium leading-snug text-text">{toast.message}</p>
        {toast.detail ? (
          <p className="mt-1 text-xs leading-relaxed text-text-secondary">{toast.detail}</p>
        ) : null}

        {toast.technical ? (
          <>
            <button
              type="button"
              onClick={() => setShowTechnical((v) => !v)}
              aria-expanded={showTechnical}
              className="mt-1.5 text-xs font-medium text-accent-text underline decoration-dotted underline-offset-2"
            >
              {showTechnical ? "Hide technical details" : "Technical details"}
            </button>
            {showTechnical ? (
              <pre className="mono mt-1.5 max-h-32 overflow-auto whitespace-pre-wrap rounded border border-border bg-surface-sunken p-2 text-[11px] leading-relaxed text-text-secondary">
                {toast.technical}
              </pre>
            ) : null}
          </>
        ) : null}

        {toast.onUndo ? (
          <Button
            size="sm"
            className="mt-2"
            icon={<Undo2 className="h-3.5 w-3.5" />}
            onClick={() => {
              toast.onUndo?.();
              onDismiss(toast.id);
            }}
          >
            Undo
          </Button>
        ) : null}
      </div>
      <IconButton label="Dismiss" size="sm" onClick={() => onDismiss(toast.id)}>
        <X className="h-3.5 w-3.5" />
      </IconButton>
    </div>
  );
}

/**
 * Split an unknown thrown value into a message worth showing and the raw text.
 *
 * Backend errors are already written for people, so they are shown as-is. Anything
 * that is not a string or an Error gets a generic message and keeps its
 * stringified form in the technical section.
 */
export function describeError(error: unknown): { message: string; technical?: string } {
  if (typeof error === "string") return { message: error };
  if (error instanceof Error) {
    return { message: error.message, technical: error.stack ?? undefined };
  }
  return {
    message: "Something went wrong.",
    technical: (() => {
      try {
        return JSON.stringify(error);
      } catch {
        return String(error);
      }
    })(),
  };
}
