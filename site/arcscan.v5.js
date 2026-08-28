/* ArcScan website behaviour. No dependencies, no tracking, no build step.
 *
 * Three jobs:
 *   1. The mobile menu.
 *   2. The screenshot switcher.
 *   3. Filling the download cards from the latest GitHub Release.
 *
 * Everything degrades. The page is fully readable and every download link works
 * before this file runs and if it never runs at all, because the markup already
 * points at the releases page. The GitHub request never blocks rendering: it is
 * fired after the page is interactive and only replaces text once it answers.
 */
(function () {
  "use strict";

  var REPO = "kingnazz/ArcScan";
  var RELEASES_LATEST = "https://github.com/" + REPO + "/releases/latest";

  // ------------------------------------------------------------- mobile menu

  var toggle = document.getElementById("nav-toggle");
  var panel = document.getElementById("nav-panel");

  if (toggle && panel) {
    var setOpen = function (open) {
      panel.setAttribute("data-open", open ? "true" : "false");
      toggle.setAttribute("aria-expanded", open ? "true" : "false");
    };

    toggle.addEventListener("click", function () {
      setOpen(panel.getAttribute("data-open") !== "true");
    });

    // Following a link closes the menu, so the target section is not hidden
    // behind it.
    panel.addEventListener("click", function (event) {
      if (event.target.closest("a")) setOpen(false);
    });

    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && panel.getAttribute("data-open") === "true") {
        setOpen(false);
        toggle.focus();
      }
    });

    // Widening past the breakpoint restores the full bar, so a menu left open
    // must not linger underneath it. 860 matches the CSS media query.
    window.addEventListener("resize", function () {
      if (window.innerWidth > 860) setOpen(false);
    });
  }

  // -------------------------------------------------------- screenshot tabs

  var tabs = Array.prototype.slice.call(document.querySelectorAll(".shot-tab"));
  var shotImage = document.getElementById("shot-image");
  var shotCaption = document.getElementById("shot-caption");
  var shotPanel = document.getElementById("shot-panel");

  if (tabs.length && shotImage && shotCaption && shotPanel) {
    var select = function (tab) {
      tabs.forEach(function (other) {
        other.setAttribute("aria-selected", other === tab ? "true" : "false");
        other.tabIndex = other === tab ? 0 : -1;
      });
      var shot = tab.getAttribute("data-shot");
      // srcset has to move with src, or a high-density screen would keep
      // painting the previous view's 2x file.
      shotImage.srcset =
        "assets/shots/" + shot + "-800.webp 800w, " +
        "assets/shots/" + shot + ".webp 1440w, " +
        "assets/shots/" + shot + "@2x.webp 2880w";
      shotImage.src = "assets/shots/" + shot + ".webp";
      shotImage.alt = tab.getAttribute("data-caption");
      shotCaption.textContent = tab.getAttribute("data-caption");
      shotPanel.setAttribute("aria-labelledby", tab.id);
    };

    tabs.forEach(function (tab, index) {
      tab.tabIndex = tab.getAttribute("aria-selected") === "true" ? 0 : -1;
      tab.addEventListener("click", function () {
        select(tab);
      });
      // Arrow keys move between tabs, which is what the tablist role promises.
      tab.addEventListener("keydown", function (event) {
        var next = null;
        if (event.key === "ArrowRight") next = tabs[(index + 1) % tabs.length];
        else if (event.key === "ArrowLeft") next = tabs[(index - 1 + tabs.length) % tabs.length];
        else if (event.key === "Home") next = tabs[0];
        else if (event.key === "End") next = tabs[tabs.length - 1];
        if (next) {
          event.preventDefault();
          select(next);
          next.focus();
        }
      });
    });
  }

  // ------------------------------------------------------------- downloads

  /** Which build this visitor most likely wants. */
  function detectPlatform() {
    var ua = navigator.userAgent || "";
    var data = navigator.userAgentData;
    var platform = data && data.platform ? data.platform.toLowerCase() : "";

    if (!platform) {
      if (/Windows/i.test(ua)) platform = "windows";
      else if (/Mac|iPhone|iPad|iPod/i.test(ua)) platform = "macos";
      else if (/Linux|Android/i.test(ua)) platform = "linux";
    }

    if (/mac|darwin|ios/i.test(platform)) return "mac";
    if (/win/i.test(platform)) {
      // Windows on ARM reports itself in the user agent; the platform hint does
      // not carry architecture without an async call, so this stays best effort
      // and every alternative download is on the page regardless.
      return /aarch64|arm64|ARM;/i.test(ua) ? "win-arm64" : "win-x64";
    }
    return null;
  }

  var recommended = detectPlatform();
  var cards = {};
  Array.prototype.forEach.call(document.querySelectorAll(".dl"), function (card) {
    cards[card.getAttribute("data-os")] = card;
  });

  var heroLink = document.getElementById("hero-download");
  var heroLabel = document.getElementById("hero-label");
  var heroGlyph = document.getElementById("hero-glyph");
  var ctaLink = document.getElementById("cta-download");
  var ctaLabel = document.getElementById("cta-label");
  var ctaGlyph = document.getElementById("cta-glyph");

  function labelFor(key) {
    if (key === "mac") return { text: "Download for macOS", glyph: "apple" };
    if (key === "win-arm64") return { text: "Download for Windows ARM64", glyph: "win" };
    if (key === "win-x64") return { text: "Download for Windows", glyph: "win" };
    return { text: "Download ArcScan", glyph: "win" };
  }

  // Mark the recommended card and set the hero button's wording immediately, so
  // the visitor sees the right platform without waiting for the network.
  (function applyPlatform() {
    var label = labelFor(recommended);
    if (heroLabel) heroLabel.textContent = label.text;
    if (heroGlyph) heroGlyph.className = "os-glyph " + label.glyph;
    if (ctaLabel) ctaLabel.textContent = label.text;
    if (ctaGlyph) ctaGlyph.className = "os-glyph " + label.glyph;

    var card = recommended && cards[recommended];
    if (card) {
      card.setAttribute("data-recommended", "true");
      var badge = card.querySelector("[data-recommended-badge]");
      if (badge) badge.hidden = false;
      var button = card.querySelector('[data-field="link"]');
      if (button) button.className = "btn btn-primary";
    }
  })();

  function formatSize(bytes) {
    var mb = bytes / (1024 * 1024);
    return mb >= 1 ? mb.toFixed(1) + " MB" : Math.round(bytes / 1024) + " KB";
  }

  /**
   * Choose one release asset for a platform and a kind of download.
   *
   * Strict on purpose. The consequence of a loose match here is a person on an
   * ARM64 machine downloading an x64 build, or somebody clicking "Download
   * installer" and getting a portable ZIP -- both of which look like a working
   * download right up until they do not work. So each rule states what the
   * name must contain *and* what it must not. The caller also supplies the
   * release tag's exact version, and anything other than one unambiguous match
   * leaves the card pointing at the release page rather than at a guess.
   */
  var ASSET_RULES = {
    "win-x64": {
      installer: {
        must: [/(^|[^a-z0-9])x64[-_]setup\.exe$/i],
        mustNot: [/arm64|aarch64/i, /portable/i],
      },
      portable: {
        must: [/^ArcScan_[\d.]+_windows-x64-portable\.zip$/i],
        mustNot: [/arm64|aarch64/i],
      },
    },
    "win-arm64": {
      installer: {
        must: [/(^|[^a-z0-9])arm64[-_]setup\.exe$/i],
        mustNot: [/portable/i],
      },
      portable: {
        must: [/^ArcScan_[\d.]+_windows-arm64-portable\.zip$/i],
        mustNot: [],
      },
    },
    mac: {
      installer: {
        must: [/universal.*\.dmg$/i],
        mustNot: [/portable/i],
      },
      // No portable macOS build in 1.8.4. Absent rather than empty-and-hidden,
      // so nothing on this page can end up implying one exists.
      portable: null,
    },
  };

  /**
   * Names that must never be offered as a download, whatever else matches.
   *
   * Updater signatures and the manifest are machinery for the in-app updater.
   * The macOS .app.tar.gz is an updater payload, not something to hand a
   * person. A GitHub source archive is not a build at all.
   */
  var NEVER = [
    /\.sig$/i,
    /^latest\.json$/i,
    /\.app\.tar\.gz$/i,
    /\.nsis\.zip$/i,
    /^(source|Source)[-_ ]?code/i,
  ];

  function pickAsset(assets, rule, expectedVersion) {
    if (!rule || !/^\d+\.\d+\.\d+(?:[-+][0-9a-z.-]+)?$/i.test(expectedVersion || "")) {
      return null;
    }
    var versionPrefix = ("ArcScan_" + expectedVersion + "_").toLowerCase();
    var matches = [];
    for (var i = 0; i < assets.length; i++) {
      var name = assets[i].name || "";
      // A stale asset left attached to a release must never satisfy a current
      // card merely because its architecture and suffix happen to match.
      if (name.toLowerCase().indexOf(versionPrefix) !== 0) continue;
      var forbidden = false;
      for (var n = 0; n < NEVER.length; n++) {
        if (NEVER[n].test(name)) forbidden = true;
      }
      if (forbidden) continue;

      var ok = rule.must.length > 0;
      for (var m = 0; m < rule.must.length; m++) {
        if (!rule.must[m].test(name)) ok = false;
      }
      for (var x = 0; x < rule.mustNot.length; x++) {
        if (rule.mustNot[x].test(name)) ok = false;
      }
      if (ok) matches.push(assets[i]);
    }
    // GitHub normally prevents duplicate names, but two differently named
    // assets may still satisfy a suffix rule. Refuse ambiguity rather than make
    // the result depend on API order.
    return matches.length === 1 ? matches[0] : null;
  }

  // Exposed for the site verification suite, which runs these rules against a
  // fixture of every asset a release publishes. Reading them from here means the
  // tests check the rules the page actually uses.
  window.__arcscanAssetRules = { rules: ASSET_RULES, never: NEVER, pick: pickAsset };

  function setField(card, field, apply) {
    var el = card.querySelector('[data-field="' + field + '"]');
    if (el) apply(el);
  }

  var status = document.getElementById("download-status");
  var releaseMeta = document.getElementById("release-meta");

  // A short timeout, because a slow or blocked API must not leave the page
  // looking like it is still loading. The markup's fallbacks already work.
  var controller = typeof AbortController === "function" ? new AbortController() : null;
  var timer = setTimeout(function () {
    if (controller) controller.abort();
  }, 6000);

  fetch("https://api.github.com/repos/" + REPO + "/releases/latest", {
    headers: { Accept: "application/vnd.github+json" },
    signal: controller ? controller.signal : undefined,
  })
    .then(function (response) {
      if (!response.ok) throw new Error("HTTP " + response.status);
      return response.json();
    })
    .then(function (release) {
      clearTimeout(timer);
      var assets = release.assets || [];
      var version = (release.tag_name || "").replace(/^v/, "");

      var map = {};
      var portableMap = {};
      Object.keys(ASSET_RULES).forEach(function (key) {
        map[key] = pickAsset(assets, ASSET_RULES[key].installer, version);
        portableMap[key] = pickAsset(assets, ASSET_RULES[key].portable, version);
      });

      if (releaseMeta && release.published_at) {
        releaseMeta.textContent =
          " · released " + new Date(release.published_at).toLocaleDateString();
      }

      Object.keys(cards).forEach(function (key) {
        var card = cards[key];
        var asset = map[key];

        if (version) setField(card, "version", function (el) { el.textContent = version; });
        if (release.html_url) {
          setField(card, "notes", function (el) { el.href = release.html_url; });
          setField(card, "checksum", function (el) { el.href = release.html_url; });
        }
        if (asset) {
          setField(card, "link", function (el) { el.href = asset.browser_download_url; });
          if (typeof asset.size === "number") {
            setField(card, "size", function (el) { el.textContent = formatSize(asset.size); });
          }
        } else {
          // No matching asset in this release: leave the control pointing at the
          // release page rather than at a link that would 404.
          setField(card, "size", function (el) { el.textContent = "See the release page"; });
        }

        var portable = portableMap[key];
        if (portable) {
          setField(card, "portable-link", function (el) {
            el.href = portable.browser_download_url;
          });
          if (typeof portable.size === "number") {
            setField(card, "portable-size", function (el) {
              el.textContent = formatSize(portable.size);
            });
          }
        } else {
          setField(card, "portable-size", function (el) {
            el.textContent = "See the release page";
          });
        }
      });

      var preferred = recommended && map[recommended];
      if (preferred) {
        if (heroLink) heroLink.href = preferred.browser_download_url;
        if (ctaLink) ctaLink.href = preferred.browser_download_url;
      }

      if (status) {
        status.textContent =
          "Sizes and links are from release " +
          (release.tag_name || "latest") +
          " on GitHub. Each asset carries a SHA-256 digest there if you want to verify a download.";
      }
    })
    .catch(function () {
      clearTimeout(timer);
      // Rate limited, offline, or blocked. Every control already points at the
      // releases page, so the only thing to change is the explanation.
      if (status) {
        status.innerHTML =
          'Release details could not be loaded from GitHub just now. Every button above opens the <a href="' +
          RELEASES_LATEST +
          '">latest release page</a>, where the installers and checksums are listed.';
      }
    });
})();
