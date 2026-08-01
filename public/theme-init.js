// Apply the saved theme before first paint so the window never flashes the
// wrong one. "system" is the default and follows the operating system.
//
// This lives in its own file rather than inline in index.html so the
// application can run under a Content Security Policy that forbids inline
// scripts. It is loaded synchronously from <head>, which is what makes it run
// before the first paint.
(function () {
  try {
    var pref = localStorage.getItem("arcscan-theme") || "system";
    var dark =
      pref === "dark" ||
      (pref === "system" &&
        window.matchMedia &&
        window.matchMedia("(prefers-color-scheme: dark)").matches);
    if (dark) document.documentElement.classList.add("dark");
  } catch (e) {
    /* storage unavailable; the app applies the theme when it mounts */
  }
})();
