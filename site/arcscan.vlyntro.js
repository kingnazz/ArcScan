/* Adds the Vlyntro build credit to the ArcScan product-site footer. */
(function () {
  "use strict";

  var wrap = document.querySelector(".footer .wrap");
  if (!wrap || wrap.querySelector(".vlyntro-credit")) return;

  var credit = document.createElement("div");
  credit.className = "vlyntro-credit";
  credit.setAttribute("aria-label", "Built by Vlyntro");
  credit.style.cssText =
    "display:flex;align-items:center;justify-content:flex-end;gap:10px;" +
    "margin-top:20px;padding-top:18px;border-top:1px solid rgba(255,255,255,.06);" +
    "font-size:12px;letter-spacing:.04em;color:#8a8a8f;";

  var label = document.createElement("span");
  label.textContent = "Built by";

  var link = document.createElement("a");
  link.href = "https://vlyntro.com";
  link.target = "_blank";
  link.rel = "noopener noreferrer";
  link.setAttribute("aria-label", "Visit Vlyntro");
  link.style.cssText =
    "display:inline-flex;align-items:center;gap:8px;color:#f5f5f7;text-decoration:none;" +
    "font-size:18px;font-weight:800;letter-spacing:-.045em;line-height:1;";

  link.innerHTML =
    '<svg viewBox="0 0 64 64" width="26" height="26" aria-hidden="true" focusable="false" style="display:block;flex:none">' +
    '<rect width="64" height="64" rx="17" fill="#2A2A2A"></rect>' +
    '<rect x="17" y="21.12" width="9" height="15" rx="4.5" fill="#FAF6F0"></rect>' +
    '<rect x="38" y="21.12" width="9" height="15" rx="4.5" fill="#FAF6F0"></rect>' +
    '<path d="M26 44.16l6 6 6-6" fill="none" stroke="#F2C94C" stroke-width="4.5" stroke-linecap="round" stroke-linejoin="round"></path>' +
    '</svg>' +
    '<span>vlyntro<span style="color:#E07856">.</span></span>';

  link.addEventListener("mouseenter", function () {
    link.style.opacity = "0.78";
  });
  link.addEventListener("mouseleave", function () {
    link.style.opacity = "1";
  });

  credit.appendChild(label);
  credit.appendChild(link);
  wrap.appendChild(credit);

  if (window.matchMedia && window.matchMedia("(max-width: 640px)").matches) {
    credit.style.justifyContent = "center";
  }
})();
