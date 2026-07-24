/* ArcScan download site logic — no dependencies, no tracking.
 * Fetch the latest GitHub release and wire real asset URLs into the download
 * controls, promoting the visitor's OS/arch. Degrades gracefully: if the API
 * is blocked/rate-limited, every control already points at releases/latest.
 */
(function () {
  "use strict";

  var REPO = "kingnazz/ArcScan";
  var RELEASES_LATEST = "https://github.com/" + REPO + "/releases/latest";

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
    if (/aarch64|arm64|ARM;/i.test(ua)) arch = "arm64";

    if (/win/i.test(platform)) return { os: "windows", arch: arch };
    if (/mac|darwin/i.test(platform)) return { os: "macos", arch: "universal" };
    return { os: "other", arch: arch };
  }

  var els = {
    primary: document.getElementById("primary-download"),
    primaryLabel: document.getElementById("primary-label"),
    primaryGlyph: document.getElementById("primary-glyph"),
    version: document.getElementById("version-label"),
    size: document.getElementById("size-label"),
    secondary: document.getElementById("secondary-downloads"),
    checksums: document.getElementById("checksums-link"),
    cta: document.getElementById("cta-download"),
    ctaLabel: document.getElementById("cta-label"),
    ctaGlyph: document.getElementById("cta-glyph"),
  };

  var plat = detectPlatform();
  applyLabels(plat, null);

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
        els.version.textContent = "Latest release " + rel.tag_name + date;
      }

      wireSecondary(map);

      var key =
        plat.os === "windows"
          ? plat.arch === "arm64"
            ? "win-arm64"
            : "win-x64"
          : plat.os === "macos"
          ? "mac"
          : "win-x64";
      var asset = map[key];

      if (asset) {
        if (els.primary) els.primary.href = asset.browser_download_url;
        if (els.cta) els.cta.href = asset.browser_download_url;
        if (els.size && typeof asset.size === "number") {
          els.size.textContent = formatSize(asset.size);
        }
        applyLabels(plat, key);
      }

      if (els.checksums && rel.html_url) els.checksums.href = rel.html_url;
    })
    .catch(function () {
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
        if (typeof asset.size === "number") links[i].title = formatSize(asset.size);
      }
    }
  }

  function applyLabels(plat, key) {
    var label, glyph;
    if (plat.os === "macos") {
      label = "Download for macOS";
      glyph = "apple";
    } else if (plat.os === "windows") {
      label = "Download for Windows" + (key === "win-arm64" ? " (ARM64)" : "");
      glyph = "win";
    } else {
      label = "Download ArcScan";
      glyph = "win";
    }
    if (els.primaryLabel) els.primaryLabel.textContent = label;
    if (els.primaryGlyph) els.primaryGlyph.className = "os-glyph " + glyph;
    if (els.ctaLabel) els.ctaLabel.textContent = label;
    if (els.ctaGlyph) els.ctaGlyph.className = "os-glyph " + glyph;
  }

  function formatSize(bytes) {
    var mb = bytes / (1024 * 1024);
    return mb >= 1 ? mb.toFixed(1) + " MB" : Math.round(bytes / 1024) + " KB";
  }
})();
