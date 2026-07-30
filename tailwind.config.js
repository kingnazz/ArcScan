/** @type {import('tailwindcss').Config} */
export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // Every colour is a token declared in index.css, so the two themes are
        // guaranteed to stay in step and nothing hardcodes a hex value.
        bg: "var(--bg)",
        surface: {
          DEFAULT: "var(--surface)",
          raised: "var(--surface-raised)",
          sunken: "var(--surface-sunken)",
          hover: "var(--surface-hover)",
          active: "var(--surface-active)",
        },
        border: {
          DEFAULT: "var(--border)",
          strong: "var(--border-strong)",
        },
        text: {
          DEFAULT: "var(--text)",
          secondary: "var(--text-secondary)",
          muted: "var(--text-muted)",
        },
        accent: {
          DEFAULT: "var(--accent)",
          hover: "var(--accent-hover)",
          fg: "var(--accent-fg)",
          text: "var(--accent-text)",
          subtle: "var(--accent-subtle)",
        },
        online: "var(--online)",
        new: "var(--new)",
        changed: "var(--changed)",
        missing: "var(--missing)",
        warning: "var(--warning)",
        danger: "var(--danger)",
        unknown: "var(--unknown)",
      },
      borderColor: {
        DEFAULT: "var(--border)",
      },
      fontFamily: {
        // System stacks, prioritising how Windows and macOS actually render.
        // No web font is downloaded, so first paint never waits on the network.
        sans: [
          "ui-sans-serif",
          "-apple-system",
          "BlinkMacSystemFont",
          "Segoe UI Variable Text",
          "Segoe UI",
          "Inter",
          "Roboto",
          "Helvetica Neue",
          "Arial",
          "sans-serif",
        ],
        mono: [
          "ui-monospace",
          "SFMono-Regular",
          "SF Mono",
          "Cascadia Mono",
          "Menlo",
          "Consolas",
          "Liberation Mono",
          "monospace",
        ],
      },
      height: {
        control: "var(--control-md)",
        "control-sm": "var(--control-sm)",
        "control-lg": "var(--control-lg)",
      },
      borderRadius: {
        sm: "var(--radius-sm)",
        md: "var(--radius-md)",
        lg: "var(--radius-lg)",
      },
      boxShadow: {
        sm: "var(--shadow-sm)",
        md: "var(--shadow-md)",
        lg: "var(--shadow-lg)",
      },
      transitionDuration: {
        fast: "var(--duration-fast)",
        base: "var(--duration-base)",
        slow: "var(--duration-slow)",
      },
      keyframes: {
        "fade-in": {
          from: { opacity: "0" },
          to: { opacity: "1" },
        },
        "slide-up": {
          from: { opacity: "0", transform: "translateY(4px)" },
          to: { opacity: "1", transform: "translateY(0)" },
        },
        "slide-in-right": {
          from: { transform: "translateX(12px)", opacity: "0" },
          to: { transform: "translateX(0)", opacity: "1" },
        },
        "row-in": {
          from: { opacity: "0.35" },
          to: { opacity: "1" },
        },
        indeterminate: {
          "0%": { transform: "translateX(-100%)" },
          "100%": { transform: "translateX(400%)" },
        },
      },
      animation: {
        "fade-in": "fade-in var(--duration-base) ease-out",
        "slide-up": "slide-up var(--duration-slow) ease-out",
        "slide-in-right": "slide-in-right var(--duration-slow) ease-out",
        // Rows fade in without moving, so a streaming table never reflows
        // under the pointer while the operator is trying to click a row.
        "row-in": "row-in var(--duration-slow) ease-out",
        indeterminate: "indeterminate 1.1s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};
