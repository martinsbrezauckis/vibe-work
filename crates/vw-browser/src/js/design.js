// The design-system capture: what the LIVE page uses, from computed styles (the engine's own resolved
// values, so CSS-in-JS, utility classes and cross-origin sheets count) and the CSS custom properties of the root.
// Usage is weighted by what is on screen: text colours by text length, backgrounds by area, borders by length.
// Heuristics adapted from ShellX Canvas (role words, type roles, scales,
// contrast); Canvas read stylesheet text of its own targets, this reads the rendered page.
// REQ: { page, maxElements }. Synchronous; the answer is untrusted page data (aggregated, every string clipped).
/* global REQ, clip, collect, domPath, editable, roleOf, viewOf -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  __VW_COMMON__
  const out = { url: String(location.href).slice(0, 2000), title: clip(document.title, 300) };
  // Transition lists use commas too, but cubic-bezier(), steps() and linear() keep their own commas.
  const firstValue = (value) => {
    let depth = 0;
    for (let i = 0; i < value.length; i += 1) {
      if (value[i] === "(") depth += 1;
      else if (value[i] === ")") depth -= 1;
      else if (value[i] === "," && depth === 0) return value.slice(0, i).trim();
    }
    return value.trim();
  };
  // Every colour is normalised through a 1x1 canvas: the engine converts any CSS colour (rgb, hsl, oklch, color(),
  // named) to sRGB pixels, so roles and contrast work on one format. Cached per string.
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  const colourCache = new Map();
  const hex2 = (n) => n.toString(16).padStart(2, "0");
  const norm = (value) => {
    const raw = String(value || "").trim();
    if (!raw || raw === "transparent" || raw === "none" || raw.startsWith("url(")) return null;
    if (colourCache.has(raw)) return colourCache.get(raw);
    let result = null;
    try {
      ctx.clearRect(0, 0, 1, 1);
      ctx.fillStyle = "rgba(0, 0, 0, 0)";
      ctx.fillStyle = raw;
      // An unparsable value leaves fillStyle unchanged (transparent): treated as "no colour".
      ctx.fillRect(0, 0, 1, 1);
      const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
      if (a > 0) {
        // getImageData answers premultiplied-then-unpremultiplied values; alpha below 1 is kept as #rrggbbaa.
        result = `#${hex2(r)}${hex2(g)}${hex2(b)}${a < 255 ? hex2(a) : ""}`;
      }
    } catch {
      result = null;
    }
    if (colourCache.size < 4000) colourCache.set(raw, result);
    return result;
  };
  const bump = (map, key, weight, extra) => {
    if (key === null || key === undefined || key === "") return;
    let row = map.get(key);
    if (!row) {
      if (map.size >= 400) return;
      row = { w: 0, n: 0, ctx: {} };
      map.set(key, row);
    }
    row.w += weight;
    row.n += 1;
    if (extra) row.ctx[extra] = (row.ctx[extra] || 0) + 1;
  };
  const rows = (map, limit) =>
    Array.from(map.entries())
      .sort((a, b) => b[1].w - a[1].w)
      .slice(0, limit)
      .map(([key, row]) => {
        const ctxs = Object.entries(row.ctx).sort((a, b) => b[1] - a[1]).slice(0, 6).map(([k]) => k);
        return { value: key, weight: Math.round(row.w), count: row.n, contexts: ctxs };
      });

  // CSS custom properties: names from every readable stylesheet (cross-origin sheets throw and are counted), values
  // as the root resolves them (var() chains resolved by the engine).
  const varNames = new Map();
  let sheetsRead = 0, sheetsBlocked = 0, rulesSeen = 0;
  const walkRules = (list) => {
    for (const rule of Array.from(list || [])) {
      if (rulesSeen >= 30000) return;
      rulesSeen += 1;
      if (rule.style) {
        for (let i = 0; i < rule.style.length && varNames.size < 600; i += 1) {
          const name = rule.style[i];
          if (name && name.startsWith("--") && !varNames.has(name)) varNames.set(name, clip(rule.selectorText, 80));
        }
      }
      if (rule.cssRules) walkRules(rule.cssRules);
    }
  };
  for (const sheet of Array.from(document.styleSheets || [])) {
    try {
      walkRules(sheet.cssRules);
      sheetsRead += 1;
    } catch {
      sheetsBlocked += 1;
    }
  }
  const rootStyle = getComputedStyle(document.documentElement);
  for (let i = 0; i < rootStyle.length && varNames.size < 600; i += 1) {
    const name = rootStyle[i];
    if (name && name.startsWith("--") && !varNames.has(name)) varNames.set(name, ":root");
  }
  const vars = [];
  for (const [name, selector] of varNames) {
    const value = clip(rootStyle.getPropertyValue(name), 160);
    if (!value) continue;
    const colour = /^(#|rgb|hsl|hwb|lab|lch|oklab|oklch|color\()/i.test(value) || /^[a-z]+$/i.test(value) ? norm(value) : null;
    vars.push({ name: clip(name, 80), value, colour, selector });
    if (vars.length >= 300) break;
  }

  const text = new Map(), backgrounds = new Map(), borders = new Map(), linkColours = new Map();
  const fonts = new Map(), spacing = new Map(), radii = new Map(), shadows = new Map(), motion = new Map();
  const pairs = new Map();
  const components = { button: new Map(), input: new Map(), card: new Map(), link: new Map() };
  const px = (v) => {
    const n = parseFloat(v);
    return Number.isFinite(n) ? n : 0;
  };
  const ownText = (el) => {
    let n = 0;
    for (const node of el.childNodes) if (node.nodeType === 3) n += String(node.textContent || "").trim().length;
    return n;
  };
  // The colour behind an element's text: the first ancestor with an opaque enough background. A background image on
  // the way makes the answer unknown (null): no contrast verdict for text on pictures.
  const backdropCache = new WeakMap();
  const backdrop = (el) => {
    let cur = el;
    const seen = [];
    let answer = undefined;
    for (let depth = 0; cur && cur.nodeType === 1 && depth < 40; depth += 1) {
      if (backdropCache.has(cur)) {
        answer = backdropCache.get(cur);
        break;
      }
      seen.push(cur);
      const cs = viewOf(cur).getComputedStyle(cur);
      if (cs.backgroundImage && cs.backgroundImage !== "none") {
        answer = null;
        break;
      }
      const bg = norm(cs.backgroundColor);
      if (bg && (bg.length === 7 || parseInt(bg.slice(7, 9), 16) >= 230)) {
        answer = bg.slice(0, 7);
        break;
      }
      cur = cur.parentElement;
    }
    // Nothing painted: the canvas colour (white unless the page asks for a dark colour scheme).
    if (answer === undefined) answer = matchMedia("(prefers-color-scheme: dark)").matches && /dark/.test(rootStyle.colorScheme) ? "#121212" : "#ffffff";
    for (const el2 of seen) backdropCache.set(el2, answer);
    return answer;
  };
  const kindOf = (el, cs) => {
    const tag = el.localName;
    const role = roleOf(el);
    if (tag === "button" || role === "button" || (tag === "input" && /^(submit|button|reset)$/i.test(el.type || ""))) return "button";
    if ((tag === "input" && editable(el)) || tag === "textarea" || tag === "select") return "input";
    if (tag === "a") {
      const bg = norm(cs.backgroundColor);
      return bg && px(cs.paddingLeft) >= 8 ? "button" : "link";
    }
    const r = el.getBoundingClientRect();
    const raised = (cs.boxShadow && cs.boxShadow !== "none") || px(cs.borderTopLeftRadius) > 0;
    if (raised && norm(cs.backgroundColor) && r.width * r.height >= 12000 && el.children.length > 0) return "card";
    return null;
  };
  const docWidth = Math.max(document.documentElement.scrollWidth, window.innerWidth, 1);
  const { elements, truncated } = collect();
  let used = 0;
  for (const el of elements) {
    if (used >= REQ.maxElements) break;
    const tag = el.localName;
    if (["script", "style", "noscript", "template", "head", "meta", "link", "title", "br"].includes(tag)) continue;
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) continue;
    const cs = viewOf(el).getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden" || Number(cs.opacity) === 0) continue;
    used += 1;
    const area = Math.min(r.width, docWidth) * Math.min(r.height, 20000);
    const chars = ownText(el);
    if (chars > 0) {
      const fg = norm(cs.color);
      const size = px(cs.fontSize);
      const weight = String(cs.fontWeight);
      bump(text, fg, chars, tag);
      if (tag === "a") bump(linkColours, fg, chars, tag);
      const family = clip(cs.fontFamily, 160);
      const lineHeight = cs.lineHeight === "normal" ? "normal" : `${Math.round(px(cs.lineHeight) * 10) / 10}px`;
      const tracking = cs.letterSpacing === "normal" ? "0" : cs.letterSpacing;
      bump(fonts, JSON.stringify([family, `${Math.round(size * 10) / 10}px`, weight, lineHeight, tracking, cs.textTransform]), chars, tag);
      const bg = backdrop(el);
      if (fg && bg) {
        const large = size >= 24 || (size >= 18.66 && Number(weight) >= 700);
        const key = `${fg.slice(0, 7)}|${bg}|${large ? "large" : "normal"}`;
        let row = pairs.get(key);
        if (!row && pairs.size < 300) {
          row = { fg: fg.slice(0, 7), bg, large, w: 0, sample: "", where: "" };
          pairs.set(key, row);
        }
        if (row) {
          row.w += chars;
          if (!row.sample) {
            row.sample = clip(el.innerText || el.textContent, 50);
            row.where = clip(domPath(el).split(" > ").slice(-3).join(" > "), 120);
          }
        }
      }
    }
    const bg = norm(cs.backgroundColor);
    if (bg) bump(backgrounds, bg, area, tag === "body" || tag === "html" ? "page" : tag);
    for (const side of ["Top", "Right", "Bottom", "Left"]) {
      const width = px(cs[`border${side}Width`]);
      if (width > 0 && cs[`border${side}Style`] !== "none") {
        bump(borders, norm(cs[`border${side}Color`]), side === "Top" || side === "Bottom" ? r.width : r.height, tag);
      }
    }
    for (const prop of ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "marginTop", "marginBottom", "rowGap", "columnGap"]) {
      const v = px(cs[prop]);
      if (v > 0 && v <= 256) bump(spacing, `${Math.round(v * 10) / 10}px`, 1, prop.replace(/(Top|Right|Bottom|Left)$/, ""));
    }
    const radius = cs.borderTopLeftRadius;
    if (px(radius) > 0) bump(radii, radius.includes("%") ? radius : `${Math.round(px(radius) * 10) / 10}px`, 1, tag);
    if (cs.boxShadow && cs.boxShadow !== "none") bump(shadows, clip(cs.boxShadow, 200), 1, tag);
    const duration = cs.transitionDuration;
    if (duration && !/^0s(,\s*0s)*$/.test(duration)) bump(motion, `${clip(firstValue(duration), 20)} ${clip(firstValue(cs.transitionTimingFunction), 60)}`, 1, tag);
    const kind = kindOf(el, cs);
    if (kind) {
      const sample = {
        backgroundColor: bg,
        textColor: norm(cs.color),
        rounded: px(radius) > 0 ? radius : "0px",
        padding: `${Math.round(px(cs.paddingTop))}px ${Math.round(px(cs.paddingRight))}px`,
        height: `${Math.round(r.height)}px`,
        fontSize: cs.fontSize,
        fontWeight: String(cs.fontWeight),
        shadow: cs.boxShadow && cs.boxShadow !== "none" ? clip(cs.boxShadow, 200) : null,
        textDecoration: kind === "link" ? clip(cs.textDecorationLine, 30) : null,
      };
      bump(components[kind], JSON.stringify(sample), 1, tag);
    }
  }
  const pageBg = norm(getComputedStyle(document.body || document.documentElement).backgroundColor) || norm(rootStyle.backgroundColor);
  out.page = {
    background: pageBg,
    colorScheme: clip(rootStyle.colorScheme, 40),
    prefersDark: matchMedia("(prefers-color-scheme: dark)").matches,
    viewport: { width: window.innerWidth, height: window.innerHeight },
    elementsSeen: used,
    truncated,
    sheetsRead,
    sheetsBlocked,
  };
  out.vars = vars;
  out.text = rows(text, 40);
  out.links = rows(linkColours, 10);
  out.backgrounds = rows(backgrounds, 40);
  out.borders = rows(borders, 20);
  out.fonts = rows(fonts, 40).map((row) => {
    const [family, size, weight, lineHeight, letterSpacing, transform] = JSON.parse(row.value);
    return { family, size, weight, lineHeight, letterSpacing, transform, chars: row.weight, count: row.count, tags: row.contexts };
  });
  out.spacing = rows(spacing, 40);
  out.radii = rows(radii, 20);
  out.shadows = rows(shadows, 12);
  out.motion = rows(motion, 10);
  out.contrast = Array.from(pairs.values())
    .sort((a, b) => b.w - a.w)
    .slice(0, 80)
    .map((p) => ({ fg: p.fg, bg: p.bg, large: p.large, weight: p.w, sample: p.sample, where: p.where }));
  out.components = {};
  for (const [kind, map] of Object.entries(components)) {
    out.components[kind] = rows(map, 4).map((row) => ({ ...JSON.parse(row.value), count: row.count }));
  }
  return JSON.stringify(out);
})()
