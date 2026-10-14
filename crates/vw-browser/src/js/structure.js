// The structure capture: the page's meaningful elements with layout boxes in document coordinates, a
// parent index (a tree of the kept elements), landmarks, headings, layout containers (flex / grid with their
// direction, columns and gap) and fixed / sticky elements. The kind classification is ShellX Canvas's,
// applied to the live DOM instead of a source map.
// Password, one-time-code, card and vault-filled fields never leave the page (as in the observation).
// REQ: { page, maxElements }. Synchronous; the answer is untrusted page data, every string clipped.
(() => {
  __VW_COMMON__
  const LANDMARK = new Set(["header", "nav", "main", "footer", "aside", "section", "article", "form"]);
  const TEXT = new Set(["h1", "h2", "h3", "h4", "h5", "h6", "p", "li", "label", "blockquote", "figcaption", "dt", "dd", "td", "th"]);
  const MEDIA = new Set(["img", "picture", "svg", "video", "audio", "canvas"]);
  const FORM = new Set(["form", "input", "textarea", "select"]);
  const sx = window.scrollX, sy = window.scrollY;
  const classesOf = (el) => Array.from(el.classList || []).slice(0, 4).map((c) => clip(c, 40));
  const kindOf = (el, cs) => {
    const tag = el.localName;
    const classes = classesOf(el).join(" ").toLowerCase();
    const role = String(el.getAttribute("role") || "").toLowerCase();
    if (tag === "nav" || role === "navigation" || /\b(nav|navbar|menu|breadcrumb)\b/.test(classes)) return "navigation";
    if (tag === "footer" || role === "contentinfo" || /\bfooter\b/.test(classes)) return "footer";
    if (tag === "header" || role === "banner") return "header";
    if (FORM.has(tag) || role === "search") return "form";
    if (MEDIA.has(tag)) return "media";
    if (tag === "button" || role === "button" || /\b(btn|button|cta)\b/.test(classes)) return "action";
    if (tag === "a") return "link";
    if (/^h[1-6]$/.test(tag)) return "heading";
    if (/\bhero\b/.test(classes)) return "hero";
    if (["main", "section", "article", "aside"].includes(tag)) return "section";
    if (TEXT.has(tag)) return "text";
    if (cs.display.includes("flex") || cs.display.includes("grid")) return "layout";
    return "container";
  };
  const keep = (el, cs, r) => {
    const tag = el.localName;
    if (LANDMARK.has(tag) || TEXT.has(tag) || MEDIA.has(tag) || FORM.has(tag)) return true;
    if (matches(el, INTERACTIVE) && tag !== "iframe" && tag !== "frame") return true;
    if (el.getAttribute("role") && el.getAttribute("role") !== "presentation" && el.getAttribute("role") !== "none") return true;
    if (cs.position === "fixed" || cs.position === "sticky") return true;
    // Layout containers that shape the page: flex/grid boxes of real size, and wide blocks with a background.
    const big = r.width >= Math.min(320, window.innerWidth * 0.25) && r.height >= 40;
    if (big && (cs.display.includes("flex") || cs.display.includes("grid")) && el.children.length >= 2) return true;
    return big && r.width >= window.innerWidth * 0.6 && cs.backgroundColor !== "rgba(0, 0, 0, 0)" && el.children.length > 0;
  };
  const nameOf = (el) => {
    const tag = el.localName;
    if (tag === "input" || tag === "textarea" || tag === "select") return labelOf(el);
    if (MEDIA.has(tag)) return clip(el.getAttribute("alt") || el.getAttribute("aria-label") || el.getAttribute("title") || "", 120);
    if (LANDMARK.has(tag) || tag === "div") return clip(el.getAttribute("aria-label") || "", 120);
    return clip(el.innerText || el.textContent, 120);
  };
  const { elements, truncated } = collect();
  const kept = [];
  const index = new Map();
  const counts = {};
  let skipped = 0;
  for (const el of elements) {
    const tag = el.localName;
    if (["script", "style", "noscript", "template", "head", "meta", "link", "title", "br", "html", "body"].includes(tag)) continue;
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) continue;
    const cs = viewOf(el).getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden") continue;
    if (!keep(el, cs, r)) continue;
    if (kept.length >= REQ.maxElements) {
      skipped += 1;
      continue;
    }
    const g = globalRect(el);
    const item = {
      i: kept.length,
      tag,
      kind: kindOf(el, cs),
      box: { x: Math.round(g.x + sx), y: Math.round(g.y + sy), width: Math.round(g.width), height: Math.round(g.height) },
    };
    let parent = el.parentElement;
    for (let depth = 0; parent && depth < 64; depth += 1) {
      if (index.has(parent)) {
        item.parent = index.get(parent);
        break;
      }
      parent = parent.parentElement;
    }
    const role = el.getAttribute("role");
    if (role) item.role = clip(role, 30);
    if (el.id) item.id = clip(el.id, 60);
    const classes = classesOf(el);
    if (classes.length) item.classes = classes;
    const secret = vaultFilled(el) ? "vault" : secretKind(el);
    if (secret) item.redacted = secret;
    const name = item.redacted && tag !== "input" && tag !== "textarea" ? "" : nameOf(el);
    if (name) item.text = name;
    if (/^h[1-6]$/.test(tag)) item.level = Number(tag[1]);
    if (cs.position === "fixed" || cs.position === "sticky") item.position = cs.position;
    if (cs.display.includes("flex")) item.layout = { display: "flex", direction: cs.flexDirection, wrap: cs.flexWrap !== "nowrap", gap: cs.gap === "normal" ? "0px" : clip(cs.gap, 30) };
    else if (cs.display.includes("grid")) {
      const columns = String(cs.gridTemplateColumns || "").split(/\s+(?![^(]*\))/).filter((c) => c && c !== "none").length;
      item.layout = { display: "grid", columns, gap: cs.gap === "normal" ? "0px" : clip(cs.gap, 30) };
    }
    if (tag === "a") {
      try {
        item.href = String(new URL(el.getAttribute("href") || "", location.href).href).slice(0, 300);
      } catch (_) {
        item.href = clip(el.getAttribute("href"), 300);
      }
    }
    if (tag === "img") item.src = clip(el.currentSrc || el.src, 300);
    if (tag === "input") item.type = clip(el.getAttribute("type") || "text", 20);
    if (inFrame(el)) item.frame = true;
    index.set(el, item.i);
    counts[item.kind] = (counts[item.kind] || 0) + 1;
    kept.push(item);
  }
  return JSON.stringify({
    url: String(location.href).slice(0, 2000),
    title: clip(document.title, 300),
    lang: clip(document.documentElement.lang, 20),
    viewport: { width: window.innerWidth, height: window.innerHeight },
    document: {
      width: Math.max(document.documentElement.scrollWidth, document.body ? document.body.scrollWidth : 0),
      height: Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0),
    },
    counts,
    truncated: truncated || skipped > 0,
    skipped,
    elements: kept,
  });
})()
