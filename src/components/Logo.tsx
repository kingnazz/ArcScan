// The ArcScan app mark, shown in the header. Uses the generated icon so the
// in-app logo matches the installer / taskbar icon exactly.

export function Logo({ size = 20 }: { size?: number }) {
  return (
    <img
      src="/icon.png"
      width={size}
      height={size}
      // Decorative: the wordmark next to it already names the app, so announcing
      // it twice is noise for a screen reader.
      alt=""
      aria-hidden
      className="rounded-[22%]"
      draggable={false}
    />
  );
}
