// The media capture's page part: what the page ALREADY loaded and shows: logo candidates, icons,
// images (in view order, with their size), CSS background images, web fonts (loaded faces and the URLs their
// @font-face rules name, kept only when the page fetched them) and inline SVG logos (markup, no fetch). The host
// fetches the URLs itself without cookies, under the navigation policy; nothing here sends a request.
// REQ: { page, maxItems }. Synchronous; the answer is untrusted page data.
(() => {
  __VW_COMMON__
  const abs = (value) => {
    try {
      const u = new URL(String(value || ""), location.href);
      return u.protocol === "http:" || u.protocol === "https:" ? u.href.slice(0, 2000) : null;
    } catch (_) {
      return null;
    }
  };
  // What the page fetched: the resource timing list (bounded); an image that finished decoding counts too (cache).
  const loaded = new Set();
  try {
    for (const entry of performance.getEntriesByType("resource").slice(0, 3000)) loaded.add(String(entry.name));
  } catch (_) {
    // No timing list: only completed images count as loaded.
  }
  const items = [];
  const seen = new Set();
  const add = (item) => {
    if (!item.url && !item.svg) return;
    const key = item.url || `svg:${item.svg.length}:${item.svg.slice(0, 200)}`;
    if (seen.has(key) || items.length >= REQ.maxItems) return;
    seen.add(key);
    items.push(item);
  };
  const logoWords = /logo|brand|wordmark/i;
  const inHeader = (el) => Boolean(el.closest && el.closest("header,nav,[role=banner]"));
  const homeLink = (el) => {
    const a = el.closest && el.closest("a[href]");
    if (!a) return false;
    try {
      const u = new URL(a.getAttribute("href"), location.href);
      return u.origin === location.origin && (u.pathname === "/" || u.pathname === "");
    } catch (_) {
      return false;
    }
  };
  const isLogo = (el) => {
    const words = ["class", "id", "alt", "src", "aria-label", "title"].map((n) => String(el.getAttribute(n) || "")).join(" ");
    const ancestorWords = el.parentElement ? String(el.parentElement.className || "") + " " + String(el.parentElement.id || "") : "";
    return logoWords.test(words) || logoWords.test(ancestorWords) || (inHeader(el) && homeLink(el));
  };
  // Freeze the live paint on a detached copy: the saved drawing has no page classes or custom properties.
  const standaloneSvg = (el) => {
    const copy = el.cloneNode(true);
    copy.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    const source = [el, ...el.querySelectorAll("*")];
    const target = [copy, ...copy.querySelectorAll("*")];
    let xlink = false;
    for (let i = 0; i < source.length; i += 1) {
      xlink ||= source[i].hasAttribute("xlink:href");
      const cs = viewOf(source[i]).getComputedStyle(source[i]);
      for (const name of ["fill", "stroke"]) {
        // Computed local paint-server URLs may be absolute; keep them local for the host's SVG allowlist.
        const paint = cs.getPropertyValue(name).replace(/url\(["']?([^"')]+)["']?\)/g, (whole, ref) => {
          try {
            const url = new URL(ref, el.ownerDocument.URL);
            return url.hash && url.href.split("#")[0] === el.ownerDocument.URL.split("#")[0] ? `url(${url.hash})` : whole;
          } catch (_) {
            return whole;
          }
        });
        if (paint) target[i].setAttribute(name, paint);
      }
    }
    if (xlink) copy.setAttribute("xmlns:xlink", "http://www.w3.org/1999/xlink");
    return copy.outerHTML;
  };
  // Icons named by the page. The browser itself fetches the first `rel=icon` (WebKitGTK does it outside the page's
  // resource list, real app run 1), so that one counts as loaded; the others only when the page fetched them.
  let firstIcon = true;
  for (const link of Array.from(document.querySelectorAll("link[rel][href]"))) {
    const rel = String(link.getAttribute("rel")).toLowerCase();
    if (!/\b(icon|apple-touch-icon|mask-icon)\b/.test(rel)) continue;
    const url = abs(link.getAttribute("href"));
    const browserIcon = firstIcon && /(^|\s)icon(\s|$)/.test(rel);
    if (browserIcon) firstIcon = false;
    add({ kind: "icon", url, rel: clip(rel, 40), sizes: clip(link.getAttribute("sizes"), 30), loaded: Boolean(url && (loaded.has(url) || browserIcon)) });
  }
  const { elements } = collect();
  const images = [];
  for (const el of elements) {
    const tag = el.localName;
    if (tag === "img") {
      const url = abs(el.currentSrc || el.src);
      if (!url || !el.complete || !el.naturalWidth) continue;
      const r = globalRect(el);
      const logo = isLogo(el);
      const row = { kind: logo ? "logo" : "image", url, alt: clip(el.getAttribute("alt"), 120), width: el.naturalWidth, height: el.naturalHeight, area: Math.round(r.width * r.height), top: Math.round(r.y + window.scrollY), loaded: true };
      if (logo) add(row);
      else images.push(row);
    } else if (tag === "svg" && isLogo(el) && !el.closest("svg svg")) {
      // An inline SVG logo is page markup: saved as text (the host removes scripts and handlers again).
      const markup = standaloneSvg(el);
      if (markup.length <= 200000) add({ kind: "logo", svg: markup, alt: clip(el.getAttribute("aria-label") || el.getAttribute("title"), 120), loaded: true });
    } else if (tag === "video" && el.poster) {
      const url = abs(el.poster);
      if (url) images.push({ kind: "image", url, alt: "video poster", area: 0, top: 0, loaded: loaded.has(url) });
    }
    if (images.length < 400 && tag !== "img") {
      let bg = "";
      try {
        bg = viewOf(el).getComputedStyle(el).backgroundImage;
      } catch (_) {
        bg = "";
      }
      if (bg && bg !== "none") {
        const m = /url\(["']?([^"')]+)["']?\)/.exec(bg);
        const url = m ? abs(m[1]) : null;
        if (url) {
          const r = globalRect(el);
          if (r.width > 0 && r.height > 0) images.push({ kind: "background", url, alt: "", area: Math.round(r.width * r.height), top: Math.round(r.y + window.scrollY), loaded: loaded.has(url) });
        }
      }
    }
  }
  images.sort((a, b) => b.area - a.area);
  for (const row of images) add(row);
  // Web fonts: faces the engine loaded, and the URLs @font-face rules name that the page fetched.
  const faces = [];
  try {
    for (const face of document.fonts || []) {
      if (faces.length >= 60) break;
      faces.push({ family: clip(face.family, 80).replace(/^["']|["']$/g, ""), weight: clip(face.weight, 20), style: clip(face.style, 20), status: clip(face.status, 12) });
    }
  } catch (_) {
    // document.fonts unavailable: the rule scan below still lists the files.
  }
  const fontUrls = [];
  const scan = (list, base) => {
    for (const rule of Array.from(list || [])) {
      if (fontUrls.length >= 80) return;
      if (rule.type === 5 && rule.style) {
        const family = clip(rule.style.getPropertyValue("font-family"), 80).replace(/^["']|["']$/g, "");
        const weight = clip(rule.style.getPropertyValue("font-weight") || "400", 20);
        const src = String(rule.style.getPropertyValue("src") || "");
        const re = /url\(["']?([^"')]+)["']?\)/g;
        let m;
        while ((m = re.exec(src))) {
          let url = null;
          try {
            url = new URL(m[1], base).href;
          } catch (_) {
            url = null;
          }
          if (url && /^https?:/.test(url)) fontUrls.push({ family, weight, url: url.slice(0, 2000) });
        }
      }
      if (rule.cssRules) scan(rule.cssRules, base);
    }
  };
  for (const sheet of Array.from(document.styleSheets || [])) {
    try {
      scan(sheet.cssRules, sheet.href || location.href);
    } catch (_) {
      // A cross-origin sheet: its font files are found through the timing list below.
    }
  }
  for (const font of fontUrls) add({ kind: "font", url: font.url, family: font.family, weight: font.weight, loaded: loaded.has(font.url) });
  for (const name of loaded) {
    if (/\.(woff2?|ttf|otf)(\?|#|$)/i.test(name)) add({ kind: "font", url: name.slice(0, 2000), family: "", weight: "", loaded: true });
  }
  return JSON.stringify({
    url: String(location.href).slice(0, 2000),
    title: clip(document.title, 300),
    faces,
    items,
    resourceCount: loaded.size,
  });
})()
