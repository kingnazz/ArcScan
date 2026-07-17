interface ToggleProps {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  description?: string;
  tone?: "default" | "warning";
  id?: string;
}

export function Toggle({ checked, onChange, label, description, tone = "default", id }: ToggleProps) {
  const activeColor = tone === "warning" ? "bg-amber-500" : "bg-brand-500";
  return (
    <label htmlFor={id} className="flex cursor-pointer items-start gap-3 select-none">
      <button
        type="button"
        role="switch"
        id={id}
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        className={`relative mt-0.5 h-5 w-9 shrink-0 rounded-full transition-colors duration-200 focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-400/50 ${
          checked ? activeColor : "bg-arc-600"
        }`}
      >
        <span
          className={`absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform duration-200 ${
            checked ? "translate-x-4" : "translate-x-0"
          }`}
        />
      </button>
      <span className="text-sm">
        <span className="font-medium text-slate-200">{label}</span>
        {description && <span className="mt-0.5 block text-xs text-slate-500">{description}</span>}
      </span>
    </label>
  );
}
