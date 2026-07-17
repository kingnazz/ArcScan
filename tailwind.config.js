/** @type {import('tailwindcss').Config} */
export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // Deep neutral canvas
        arc: {
          950: "#080b12",
          900: "#0b0f17",
          850: "#0f141f",
          800: "#141b28",
          700: "#1c2536",
          600: "#273349",
          500: "#3a4a67",
          400: "#5a6d8f",
        },
        // Brand accent — cyan/teal arc
        brand: {
          50: "#e6fbff",
          100: "#b8f3ff",
          200: "#7ce7fb",
          300: "#38d4f0",
          400: "#12b8db",
          500: "#0898b8",
          600: "#0a7896",
          700: "#0f5f78",
          800: "#154d61",
          900: "#163f50",
        },
      },
      fontFamily: {
        sans: [
          "Inter",
          "system-ui",
          "-apple-system",
          "Segoe UI",
          "Roboto",
          "Helvetica Neue",
          "Arial",
          "sans-serif",
        ],
        mono: [
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "Consolas",
          "Liberation Mono",
          "monospace",
        ],
      },
      boxShadow: {
        glow: "0 0 0 1px rgba(56,212,240,0.15), 0 8px 30px -8px rgba(8,152,184,0.35)",
        panel: "0 1px 0 0 rgba(255,255,255,0.03) inset, 0 12px 40px -18px rgba(0,0,0,0.8)",
      },
      keyframes: {
        "fade-in": {
          from: { opacity: "0", transform: "translateY(4px)" },
          to: { opacity: "1", transform: "translateY(0)" },
        },
        "pulse-ring": {
          "0%": { transform: "scale(0.8)", opacity: "0.8" },
          "100%": { transform: "scale(2.2)", opacity: "0" },
        },
      },
      animation: {
        "fade-in": "fade-in 0.25s ease-out",
        "pulse-ring": "pulse-ring 1.4s ease-out infinite",
      },
    },
  },
  plugins: [],
};
