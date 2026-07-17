// The ArcScan app mark, shown in the header. Uses the generated icon so the
// in-app logo matches the installer / taskbar icon exactly.

export function Logo({ size = 28 }: { size?: number }) {
  return (
    <img
      src="/icon.png"
      width={size}
      height={size}
      alt="ArcScan"
      className="rounded-[22%] shadow-soft"
      draggable={false}
    />
  );
}
