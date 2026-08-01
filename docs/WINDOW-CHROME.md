# Window chrome decision (v1.7.1)

ArcScan uses **Option A: native decorations**, now stated explicitly in
`src-tauri/tauri.conf.json` (`"decorations": true`) rather than relied on as a
default.

## Why

The in-app 38 px header is a **toolbar**, not a title-bar replacement: it holds
the view switcher (Devices / Changes / History) and window-level actions
(updates, theme, settings) and renders **no** minimize/maximize/close controls.
With the native title bar present there is therefore no duplicated control and
no missing one — the OS provides the window controls, snapping, full-screen
behaviour, keyboard window management and accessibility hooks, which is exactly
the set of behaviours a fully custom title bar would otherwise have to
re-implement per platform.

The `data-tauri-drag-region` attributes on the toolbar are a supplementary drag
surface: Tauri only starts a window drag when the press lands on the attributed
element itself, so the tabs and buttons inside the bar keep working normally.
Double-clicking empty toolbar space toggles maximize, matching what the same
gesture does on the native bar above it.

## Packaged-build verification checklist

Browser previews cannot exercise native window chrome. The following must be
verified on a **packaged** build before release, on Windows 11 and macOS:

- [ ] Exactly one title bar is visible (the native one above the toolbar)
- [ ] Minimize, maximize, restore and close all work from the native controls
- [ ] Window dragging works from the native bar *and* from empty toolbar space
- [ ] Double-click on the title bar and on empty toolbar space toggles maximize
- [ ] Windows: Snap layouts appear on hovering the maximize button
- [ ] macOS: traffic lights sit in the native bar with standard placement
- [ ] Full-screen (macOS green button / Windows maximize) and return
- [ ] Keyboard window controls (Win+arrows; macOS window menu shortcuts)
- [ ] Resizing from every edge and corner
- [ ] The 940×620 minimum window size is enforced

If any check fails, the fallback is to remove the toolbar's drag-region
attributes (pure convenience) rather than to move to custom decorations.
