// The observation of the agent browser: what the page shows as a compact model with refs for its
// interactive elements. Values of password, one-time-code and payment-card fields and of fields the vault filled
// never leave the page (`redacted`); every other value is readable. Text, names,
// values and links that look like an API key are hidden (`keyLike`). Text is the page's visible text (layout-aware innerText), one window of it per
// call (`REQ.textOffset`, `REQ.maxText`).
// Hidden text is returned separately (bounded) for the prompt guard only.
// REQ: { page, maxRefs, maxText, textOffset }. Synchronous; the answer is untrusted page data.
(() => {
  __VW_COMMON__
  const everything = collect();
  for (const el of everything.elements) rememberLabelledKeys(el);
  // The original address stays on the host for admission checks; only displayUrl goes into agent answers.
  const out = { url: String(location.href).slice(0, 2000), displayUrl: hideKeys(String(location.href)).slice(0, 2000), title: clip(hideKeys(document.title), 300), readyState: document.readyState };
  const { truncated, crossOriginFrames } = everything;
  // Show usable controls before hidden tab/panel copies.
  const elements = everything.elements.filter(actionableCandidate)
    .map((el) => ({ el, shown: visible(el) }))
    .sort((a, b) => Number(b.shown) - Number(a.shown)).map(({ el }) => el);
  const refs = [];
  const frames = [];
  const seen = new Set();
  // Elements that show a key come first (never trimmed away), named and valued as hidden.
  for (const el of everything.elements) {
    if (refs.length >= REQ.maxRefs) break;
    if (!keyHolder(el)) continue;
    const ref = refOf(el);
    if (seen.has(ref)) continue;
    seen.add(ref);
    const item = { ref, role: actionableCandidate(el) ? roleOf(el) : "text", name: hideKeys(labelOf(el), el), tag: el.localName, keyLike: true, value: KEY_TEXT };
    if (!visible(el)) item.visible = false;
    refs.push(item);
  }
  for (const el of elements) {
    const ref = refOf(el);
    if (seen.has(ref)) continue;
    seen.add(ref);
    const role = roleOf(el);
    const label = labelOf(el);
    const item = { ref, role, name: hideKeys(label, el), tag: el.localName };
    if (keyLike(label, el)) item.keyLike = true;
    if (role === "iframe") {
      let src = "";
      try {
        const u = new URL(el.getAttribute("src") || "about:blank", location.href);
        src = u.origin === "null" ? u.href : u.origin + u.pathname;
      } catch (_) {
        src = String(el.getAttribute("src") || "");
      }
      frames.push({ ref, name: item.name, src: clip(hideKeys(src), 200), sameOrigin: Boolean(frameDoc(el)), visible: visible(el) });
      continue;
    }
    if (refs.length >= REQ.maxRefs) continue;
    if (inFrame(el)) item.frame = true;
    const isVisible = visible(el);
    if (!isVisible) item.visible = false;
    else {
      const r = globalRect(el);
      if (r.y + r.height < 0 || r.y > window.innerHeight || r.x + r.width < 0 || r.x > window.innerWidth) item.offscreen = true;
    }
    if (!enabled(el)) item.disabled = true;
    const tag = el.localName;
    if (tag === "input" || tag === "textarea" || el.isContentEditable || tag === "select") {
      const type = String(el.getAttribute("type") || "").toLowerCase();
      if (type) item.type = clip(type, 20);
      // A sign-in field's role, and (host side only, never in the agent's answer) where its form submits.
      const role = loginRole(el);
      if (role) {
        item.login = role;
        item.submitsTo = submitOrigins(el);
      }
      // Only password, one-time-code, card and vault-filled fields are redacted, whatever their sign-in role.
      // Every other value is readable, also the user name and text the user typed.
      const secret = vaultFilled(el) ? "vault" : secretKind(el);
      if (secret) item.redacted = secret;
      if (type === "checkbox" || type === "radio") item.checked = Boolean(el.checked);
      else if (tag === "select") {
        const options = Array.from(el.options || []);
        item.value = clip(hideKeys(options.filter((o) => o.selected).map((o) => o.label || o.text).join(", "), el), 200);
        item.options = options.slice(0, 20).map((o) => clip(hideKeys(o.label || o.text || o.value, el), 80));
        if (keyLike(el.value, el)) item.keyLike = true;
        if (options.length > 20) item.optionsMore = options.length - 20;
      } else if (!item.redacted && !el.isContentEditable) {
        item.value = clip(hideKeys(el.value, el), 300);
        if (keyLike(el.value, el)) item.keyLike = true;
      }
      if (el.required) item.required = true;
    } else {
      const checked = el.getAttribute("aria-checked");
      if (checked !== null) item.checked = checked === "true";
    }
    const expanded = el.getAttribute("aria-expanded");
    if (expanded !== null) item.expanded = expanded === "true";
    const pressed = el.getAttribute("aria-pressed");
    if (pressed !== null) item.pressed = pressed === "true";
    if (tag === "a") {
      try {
        item.href = hideKeys(String(new URL(el.getAttribute("href"), location.href).href)).slice(0, 300);
      } catch (_) {
        item.href = clip(hideKeys(el.getAttribute("href")), 300);
      }
    }
    refs.push(item);
  }
  // Visible page text, including any dialog text.
  let text = "";
  try { text = String(document.body?.innerText || ""); } catch (_) { text = ""; }
  // Text that looks like a key is hidden (the element that shows it has a keyLike ref).
  text = hideKeys(text);
  const offset = Math.max(0, Math.min(Number(REQ.textOffset) || 0, text.length));
  out.text = text.slice(offset, offset + REQ.maxText);
  out.textOffset = offset;
  out.textLength = text.length;
  // Hidden text (display:none, visibility:hidden, tiny, off-canvas): only for the prompt guard.
  const hidden = [];
  let hiddenLength = 0;
  try {
    const walker = document.createTreeWalker(document.body || document.documentElement, 4);
    for (let node = walker.nextNode(); node && hiddenLength < 3000; node = walker.nextNode()) {
      const value = String(node.textContent || "").trim();
      const parent = node.parentElement;
      if (!value || !parent || ["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE"].includes(parent.tagName)) continue;
      const style = getComputedStyle(parent);
      const rect = parent.getBoundingClientRect();
      const offCanvas = rect.right < -100 || rect.bottom < -100;
      const tiny = parseFloat(style.fontSize) < 2;
      const invisible = style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0;
      if (invisible || tiny || offCanvas || parent.getAttribute("aria-hidden") === "true") {
        hidden.push(value.slice(0, 500));
        hiddenLength += Math.min(value.length, 500);
      }
    }
  } catch (_) {
    // A page that blocks the walk only loses the hidden-text check for this observation.
  }
  out.hiddenText = hideKeys(hidden.join("\n")).slice(0, 3000);
  out.refs = refs;
  out.allRefs = Array.from(seen).slice(0, 1000);
  out.refCount = seen.size - frames.length;
  out.frames = frames.slice(0, 20);
  out.crossOriginFrames = crossOriginFrames;
  out.truncated = truncated;
  out.scroll = {
    y: Math.round(window.scrollY),
    height: Math.round((document.documentElement && document.documentElement.scrollHeight) || 0),
    viewport: Math.round(window.innerHeight),
  };
  out.environment = Boolean(ENV);
  return JSON.stringify(out);
})()
