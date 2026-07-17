// Inline SVG version of the ArcScan radar mark, used in the header.

export function Logo({ size = 28 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      <defs>
        <linearGradient id="arc-bg" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#141b28" />
          <stop offset="1" stopColor="#080b12" />
        </linearGradient>
        <linearGradient id="arc-sweep" x1="0" y1="1" x2="1" y2="0">
          <stop offset="0" stopColor="#0a7896" stopOpacity="0.15" />
          <stop offset="1" stopColor="#38d4f0" stopOpacity="0.9" />
        </linearGradient>
      </defs>
      <rect x="1" y="1" width="62" height="62" rx="15" fill="url(#arc-bg)" stroke="#1c2536" />
      <g stroke="#0a7896" strokeWidth="1.6" fill="none" opacity="0.8">
        <path d="M22 46 A 16 16 0 0 1 38 30" />
        <path d="M22 46 A 26 26 0 0 1 48 24" />
        <path d="M22 46 A 36 36 0 0 1 57 20" />
      </g>
      <path d="M22 46 L 46 22 L 40 46 Z" fill="url(#arc-sweep)" />
      <circle cx="24" cy="20" r="2.4" fill="#96f0ff" />
      <circle cx="44" cy="34" r="2" fill="#7ce7fb" />
      <circle cx="45" cy="19" r="1.6" fill="#7ce7fb" />
    </svg>
  );
}
