// Builds a passive document from the sanitized DOM and the live CSSOM. Called by page.js.
// The host packs only resource URLs observed loaded by this document, using the existing capture fetcher.
/* global REQ -- Supplied by the Rust script assembler and its shared prelude. */
/* exported singleHtml */
const singleHtml = (copy, root) => {
  const assets = [];
  const loaded = new Set(performance.getEntriesByType("resource").map((r) => r.name));
  const absolute = (url, base = document.baseURI) => {
    try { return new URL(url, base).href; } catch { return ""; }
  };
  const add = (url, base, ready = false, css = false) => {
    const href = absolute(url, base);
    if (!href || href.startsWith("#")) return href;
    const known = assets.find((a) => a.url === href && a.css === css);
    if (known) { known.loaded ||= ready; return known.token; }
    const token = `vw-packed-${REQ.token}-${assets.length}-end`;
    assets.push({ url: href, token, css, loaded: ready || loaded.has(href) || href.startsWith("data:") });
    return token;
  };
  const urls = (css, base, raw = true) => css.replace(/url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*))\s*\)/gi,
    (_, a, b, c) => {
      const value = (a ?? b ?? c ?? "").trim();
      return value.startsWith("vw-packed-") ? `url("${value}")` : value.startsWith("#") ? `url("${value}")` : `url("${add(value, base, false, raw)}")`;
    });
  let inaccessible = false;
  const rules = (sheet, seen = new Set()) => {
    if (seen.has(sheet) || seen.size > 100) return "";
    seen.add(sheet);
    let list;
    try { list = Array.from(sheet.cssRules); } catch { inaccessible = true; return ""; }
    return list.map((rule) => {
      if (rule.type === 3) {
        const content = rule.styleSheet ? rules(rule.styleSheet, seen) : "";
        return rule.media?.mediaText ? `@media ${rule.media.mediaText}{${content}}` : content;
      }
      if (rule.selectorText) {
        try { if (!document.querySelector(rule.selectorText) && !rule.selectorText.includes(":")) return ""; } catch { /* Keep selectors the query API cannot interpret. */ }
      }
      return urls(rule.cssText, sheet.href || document.baseURI);
    }).join("\n");
  };
  const css = Array.from(document.styleSheets).map((s) => {
    const text = rules(s);
    return s.media?.mediaText ? `@media ${s.media.mediaText}{${text}}` : text;
  }).join("\n");
  // A cross-origin sheet may hide its CSSOM. Its used declarations remain readable through computed style.
  if (inaccessible) {
    const originals = [root, ...root.querySelectorAll("*")];
    const clones = [copy, ...copy.querySelectorAll("*")];
    // Sanitization removes nodes; pair by a temporary marker assigned before it runs.
    for (const el of clones) {
      const index = Number(el.getAttribute("data-vw-pack-index"));
      const original = originals[index];
      if (!original) continue;
      const style = getComputedStyle(original);
      el.setAttribute("style", Array.from(style).map((key) => `${key}:${urls(style.getPropertyValue(key), document.baseURI, false)}`).join(";"));
    }
  }
  for (const el of copy.querySelectorAll("[style]")) el.setAttribute("style", urls(el.getAttribute("style"), document.baseURI, false));
  for (const el of copy.querySelectorAll("style,link[rel~=stylesheet]")) el.remove();
  const style = document.createElement("style");
  style.textContent = css.replace(/<\/style/gi, "<\\/style");
  (copy.querySelector("head") || copy).append(style);
  const originals = [root, ...root.querySelectorAll("*")];
  for (const el of [copy, ...copy.querySelectorAll("*")]) {
    const index = Number(el.getAttribute("data-vw-pack-index"));
    const original = originals[index];
    el.removeAttribute("data-vw-pack-index");
    if (el.localName === "img") {
      const url = original?.currentSrc || original?.src || el.getAttribute("src");
      if (url) el.setAttribute("src", add(url, document.baseURI, Boolean(original?.complete && original?.naturalWidth)));
      el.removeAttribute("loading");
    } else if (el.localName === "source") {
      el.remove(); // The picture's chosen source is already on its img.
    } else if (el.localName === "link" && el.hasAttribute("href")) {
      if (el.rel.includes("icon")) el.setAttribute("href", add(el.href, document.baseURI));
      else el.remove();
    }
    if (el.hasAttribute("poster")) el.setAttribute("poster", add(el.getAttribute("poster"), document.baseURI));
    if (["image", "use"].includes(el.localName)) for (const attr of ["href", "xlink:href"]) {
      const url = el.getAttribute(attr);
      if (url && !url.startsWith("#")) el.setAttribute(attr, add(url, document.baseURI));
    }
  }
  return JSON.stringify({ html: "<!DOCTYPE html>\n" + copy.outerHTML, assets });
};
