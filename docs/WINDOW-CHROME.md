# Window chrome decision

ArcScan uses **Option A: native decorations**, stated explicitly in
`src-tauri/tauri.conf.json` (`"decorations": true`) rather than relied on as a
default.

## Why

The in-app 38 px header is a **toolbar**, not a title-bar replacement: it holds
the view switcher (Scan / Inventory / Changes / History) and window-level
actions (updates, theme, settings) and renders **no** minimize/maximize/close
controls. With the native title bar present there is therefore no duplicated
control and no missing one — the OS provides the window controls, snapping,
full-screen behaviour, keyboard window management and accessibility hooks,
which is exactly the set of behaviours a fully custom title bar would otherwise
have to re-implement per platform.

The `data-tauri-drag-region` attributes on the toolbar are a supplementary drag
surface: Tauri only starts a window drag when the press lands on the attributed
element itself, so the tabs and buttons inside the bar keep working normally.
Double-clicking empty toolbar space toggles maximize, matching what the same
gesture does on the native bar above it.

## The duplicate title, and its removal in 1.8.1

Up to and including 1.8.0 the toolbar also rendered an ArcScan icon and the
word "ArcScan" at the start of the row. Because the window is decorated by the
OS and `tauri.conf.json` sets `"title": "ArcScan"`, the packaged app therefore
showed the name and the mark **twice**, stacked: once in the native title bar
and once immediately below it. This is invisible in the browser preview, which
has no native title bar at all, which is why it survived several releases.

1.8.1 removes the in-app brand block (`Logo` + wordmark) rather than the native
title:

- the native title bar is the one an operating system already positions,
  themes, and exposes to accessibility tooling;
- a custom title bar would have to re-implement minimize/maximize/restore,
  snapping, full screen and keyboard window management on two platforms, which
  is explicitly out of scope;
- removing the block gives the view switcher the start of the row, so
  navigation is now the first thing in the app.

`src/components/TitleBar.tsx` was renamed to `src/components/AppHeader.tsx` so
the name stops implying a responsibility the component does not have, and
`src/components/Logo.tsx` was deleted because nothing else used it.
`public/icon.png` stays: it is still the favicon for the browser build.

## Packaged-build verification checklist

Browser previews cannot exercise native window chrome. The following must be
verified on a **packaged** build before release, on Windows 11 and macOS:

- [ ] Exactly one title bar is visible (the native one above the toolbar)
- [ ] Exactly one ArcScan icon and one ArcScan title are visible
- [ ] No blank strip where the in-app brand block used to be
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
