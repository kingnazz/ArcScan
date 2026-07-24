/* ArcScan download site logic — no dependencies, no tracking.
 * 1. Theme toggle (persisted, respects prefers-color-scheme).
 * 2. Fetch the latest GitHub release and wire real asset URLs into the
 *    download controls, promoting the visitor's OS/arch.
 * 3. Degrade gracefully: if the API is blocked/rate-limited, every control
 *    already points at releases/latest from the static HTML.
 */
(function () {
  "use strict";

  var REPO = "kingnazz/ArcScan";
  var RELEASES_LATEST = "https://github.com/" + REPO + "/releases/latest";

  /* ---------- Theme ---------- */
  var root = document.documentElement;
  var stored = null;
  try {
    stored = localStorage.getItem("arcscan-site-theme");
  } catch (e) {
    /* ignore */
  }
  var prefersDark =
    window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
  setTheme(stored || (prefersDark ? "dark" : "light"));

  var toggle = document.getElementById("theme-toggle");
  if (toggle) {
    toggle.addEventListener("click", function () {
      setTheme(root.getAttribute("data-theme") === "dark" ? "light" : "dark");
    });
  }

  function setTheme(theme) {
    root.setAttribute("data-theme", theme);
    try {
      localStorage.setItem("arcscan-site-theme", theme);
    } catch (e) {
      /* ignore */
    }
    // Swap the hero screenshot to match the theme.
    var hero = document.getElementById("hero-image");
    if (hero) {
      var src = hero.getAttribute(theme === "dark" ? "data-dark" : "data-light");
      if (src) hero.src = src;
    }
  }

  /* ---------- OS / arch detection ---------- */
  function detectPlatform() {
    var ua = navigator.userAgent || "";
    var uaData = navigator.userAgentData;
    var platform = "";
    var arch = "x64";

    if (uaData && uaData.platform) {
      platform = uaData.platform.toLowerCase();
    } else if (/Windows/i.test(ua)) {
      platform = "windows";
    } else if (/Mac|iPhone|iPad|iPod/i.test(ua)) {
      platform = "macos";
    } else if (/Linux|Android/i.test(ua)) {
      platform = "linux";
    }

    // ARM hints (best-effort; UA rarely exposes Windows-on-ARM reliably).
    if (/aarch64|arm64|ARM;/i.test(ua)) arch = "arm64";

    if (/win/i.test(platform)) return { os: "windows", arch: arch };
    if (/mac|darwin/i.test(platform)) return { os: "macos", arch: "universal" };
    return { os: "other", arch: arch };
  }

  /* ---------- Release fetch ---------- */
  var els = {
    primary: document.getElementById("primary-download"),
    primaryLabel: document.getElementById("primary-label"),
    primaryGlyph: document.getElementById("primary-glyph"),
    version: document.getElementById("version-label"),
    size: document.getElementById("size-label"),
    secondary: document.getElementById("secondary-downloads"),
    checksums: document.getElementById("checksums-link"),
  };

  var plat = detectPlatform();
  // Set a sensible primary label immediately, before the API responds.
  applyPrimaryLabel(plat, null);

  fetch("https://api.github.com/repos/" + REPO + "/releases/latest", {
    headers: { Accept: "application/vnd.github+json" },
  })
    .then(function (r) {
      if (!r.ok) throw new Error("HTTP " + r.status);
      return r.json();
    })
    .then(function (rel) {
      var assets = rel.assets || [];
      var map = {
        "win-x64": pickAsset(assets, /x64-setup\.exe$/i),
        "win-arm64": pickAsset(assets, /arm64-setup\.exe$/i),
        mac: pickAsset(assets, /universal\.dmg$/i) || pickAsset(assets, /\.dmg$/i),
      };

      if (els.version && rel.tag_name) {
        var date = rel.published_at
          ? " · " + new Date(rel.published_at).toLocaleDateString()
          : "";
        els.version.textContent = "Latest: " + rel.tag_name + date;
      }

      // Wire secondary links to real asset URLs where available.
      wireSecondary(map);

      // Choose the primary target for this visitor.
      var primaryKey =
        plat.os === "windows"
          ? plat.arch === "arm64"
            ? "win-arm64"
            : "win-x64"
          : plat.os === "macos"
          ? "mac"
          : "win-x64"; // sensible default for unknown OS
      var primaryAsset = map[primaryKey];

      if (primaryAsset) {
        els.primary.href = primaryAsset.browser_download_url;
        if (els.size && typeof primaryAsset.size === "number") {
          els.size.textContent = formatSize(primaryAsset.size);
        }
        applyPrimaryLabel(plat, primaryKey);
      }

      // Checksums live on the release page (and/or a checksums asset).
      if (els.checksums && rel.html_url) els.checksums.href = rel.html_url;
    })
    .catch(function () {
      // Static HTML already points everything at releases/latest — nothing to do
      // except make the version line non-blocking.
      if (els.version) {
        els.version.innerHTML =
          'Latest release on <a href="' + RELEASES_LATEST + '">GitHub</a>';
      }
    });

  function pickAsset(assets, re) {
    for (var i = 0; i < assets.length; i++) {
      if (re.test(assets[i].name)) return assets[i];
    }
    return null;
  }

  function wireSecondary(map) {
    if (!els.secondary) return;
    var links = els.secondary.querySelectorAll("a[data-os]");
    for (var i = 0; i < links.length; i++) {
      var key = links[i].getAttribute("data-os");
      var asset = map[key];
      if (asset) {
        links[i].href = asset.browser_download_url;
        if (typeof asset.size === "number") {
          links[i].title = formatSize(asset.size);
        }
      }
    }
  }

  function applyPrimaryLabel(plat, key) {
    if (!els.primaryLabel || !els.primaryGlyph) return;
    var label, glyph;
    if (plat.os === "macos") {
      label = "Download for macOS";
      glyph = "apple";
    } else if (plat.os === "windows") {
      label =
        "Download for Windows" + (key === "win-arm64" ? " (ARM64)" : " (x64)");
      glyph = "win";
    } else {
      label = "Download ArcScan";
      glyph = "win";
    }
    els.primaryLabel.textContent = label;
    els.primaryGlyph.className = "os-glyph " + glyph;
  }

  function formatSize(bytes) {
    var mb = bytes / (1024 * 1024);
    return mb >= 1 ? mb.toFixed(1) + " MB" : Math.round(bytes / 1024) + " KB";
  }
})();
