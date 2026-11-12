// The observation of the agent browser: what the page shows as a compact model with refs for its
// interactive elements. Values of password, one-time-code and payment-card fields and of fields the vault filled
// never leave the page (`redacted`); every other value is readable. Text, names,
// values and links that look like an API key are hidden (`keyLike`). Text is the page's visible text (layout-aware innerText), one window of it per
// call (`REQ.textOffset`, `REQ.maxText`).
// Hidden text is returned separately (bounded) for the prompt guard only.
// REQ: { page, maxRefs, maxText, textOffset, find?, node? }. Synchronous; the answer is untrusted page data.
/* global choiceContextFor, overlayChoiceRoots, nodeIdentity, ENV, KEY_TEXT, REQ, extractData, safeSelectors, actionableCandidate, clip, collect, controlSearchText, formControl, enabled, frameDoc, globalRect, hideKeys, inFrame, keyHolder, keyLike, labelOf, labelledToggle, loginRole, privateField, refOf, rememberLabelledKeys, roleOf, secretKind, submitOrigins, vaultFilled, visible, visualCandidate -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  __VW_COMMON__
  // eslint-disable-next-line no-undef -- Replaced by the Rust observation assembler.
  __VW_SELECTORS__
  __VW_EXTRACT__
  const everything = collect(!REQ.selector && !REQ.extract);
  if (ENV && !ENV.frameDocument) Object.defineProperty(ENV, "frameDocument", { value:
    Array.from(crypto.getRandomValues(new Uint32Array(4)), n => n.toString(16)).join("-") });
  for (const el of everything.elements) rememberLabelledKeys(el);
  const scopeRoots = REQ.scope ? everything.elements.filter(el => refOf(el) === REQ.scope.ref && nodeIdentity(el) === REQ.scope.node && (!REQ.scope.document || REQ.scope.document === ENV?.frameDocument)) : [];
  const scopeRoot = scopeRoots.length === 1 ? scopeRoots[0] : null;
  if (REQ.scope) everything.elements = everything.elements.filter(el => {
    for (let up = el; up; up = up.parentElement || up.getRootNode()?.host) if (up === scopeRoot) return true;
    return false;
  });
  const protectedEditor = everything.elements.some(el => el.isContentEditable && privateField(el));
  const select = REQ.selector || REQ.extract || scopeRoot || protectedEditor ? safeSelectors(everything.elements) : null;
  if (select?.truncated) everything.truncated = true;
  // The original address stays on the host for admission checks; only displayUrl goes into agent answers.
  const out = { url: String(location.href).slice(0, 2000), displayUrl: hideKeys(String(location.href)).slice(0, 2000), title: clip(hideKeys(document.title), 300), readyState: document.readyState };
  out.document = ENV?.frameDocument || "";
  out.scopeMatches = scopeRoots.length;
  if (REQ.extract) Object.assign(out, extractData(everything.elements, everything.truncated, select));
  const headings = everything.elements.filter(el => el.ownerDocument === document &&
    el.matches('h1,[role="heading"][aria-level="1"]') && visible(el));
  out.mainHeadings = headings.slice(0, 3).map(el => clip(hideKeys(select ? select.text(el) : el.innerText || el.textContent, el), 180));
  out.mainHeadingsTotal = headings.length;
  const { truncated, crossOriginFrames } = everything;
  // Selector filters run before ref caps and use node identity, including open shadow roots.
  let selected = null;
  if (REQ.selector) {
    selected = new Set();
    try {
      selected = select(REQ.selector.kind, REQ.selector.value);
      selected = new Set(everything.elements.filter(el => selected.has(el) && !el.matches("script,style,noscript,template")));
      out.selectorMatches = selected.size;
    } catch { out.selectorError = "invalid"; out.selectorMatches = 0; selected.clear(); }
  }
  const needle = typeof REQ.find === "string" ? REQ.find.toLowerCase() : null;
  // Search only redacted display fields, never the original value of a secret field. A checked click's fresh
  // observation (`REQ.node`) looks only for the node it clicks, so the ref cap never hides it.
  const focusedMatch = item => typeof REQ.node === "number" ? item.node === REQ.node : needle === null ||
    [item.name, item.role, item.value, item.searchText].some(value => String(value || "").toLowerCase().includes(needle));
  // The fallback reader enters same-origin children itself. Measure their allocation from parent
  // elements, clipped through every ancestor, using the same 40% threshold as the native merger.
  const frameLimits = new WeakMap();
  const frameLimit = el => {
    if (!inFrame(el)) return 7;
    const owner = el.ownerDocument;
    if (frameLimits.has(owner)) return frameLimits.get(owner);
    let left = 0, top = 0, right = innerWidth, bottom = innerHeight, doc = owner;
    while (doc !== document) {
      const frame = doc.defaultView?.frameElement;
      if (!frame) { right = left; break; }
      const r = globalRect(frame);
      const sx = frame.offsetWidth ? r.width / frame.offsetWidth : 0;
      const sy = frame.offsetHeight ? r.height / frame.offsetHeight : 0;
      const x = r.x + frame.clientLeft * sx, y = r.y + frame.clientTop * sy;
      left = Math.max(left, x); top = Math.max(top, y);
      right = Math.min(right, x + frame.clientWidth * sx);
      bottom = Math.min(bottom, y + frame.clientHeight * sy);
      doc = frame.ownerDocument;
    }
    const area = innerWidth * innerHeight;
    const limit = area > 0 && Math.max(0, right - left) * Math.max(0, bottom - top) >= area * 0.4 ? 7 : 3;
    frameLimits.set(owner, limit);
    return limit;
  };
  const priority = el => {
    if (!visible(el)) return -1;
    const r = globalRect(el);
    const onScreen = r.y + r.height > 0 && r.y < innerHeight && r.x + r.width > 0 && r.x < innerWidth;
    for (let up = el; up; up = up.parentElement || up.getRootNode()?.host) {
      if (onScreen && up.matches('[role="dialog"], [aria-modal="true"], dialog[open]')) return 6;
      const position = getComputedStyle(up).position;
      if (onScreen && (position === "fixed" || position === "sticky")) return 5;
    }
    if (formControl(el)) return 4;
    if (el.closest('main,article,[role=main],form') && !el.closest('nav,[role=navigation]')) return 3;
    return onScreen ? 1 : 0;
  };
  // Overlays first, then rendered form controls and main content, even below the fold, before navigation.
  const elements = everything.elements.filter(el => selected ? selected.has(el) : actionableCandidate(el) || visualCandidate(el))
    .map((el) => ({ el, rank: selected ? 0 : refOf(el) === REQ.priorityRef ? 8 : Math.min(needle !== null && labelOf(el).toLowerCase() === needle ? 7 : priority(el), frameLimit(el)) }))
    .sort((a, b) => b.rank - a.rank);
  // A consent dialog in this document needs the same independent inventory as a covering child frame.
  // Find its roots before the ordinary page-choice cap: unrelated links cannot hide a late dialog.
  const overlays = new Map((REQ.scope ? [] : overlayChoiceRoots(elements.filter(({ el }) =>
    el.ownerDocument === document && visible(el)))).slice(0, 4)
    .map(root => [root, { choices: [], total: 0 }]));
  const refs = [];
  const choiceGroups = new Map();
  // Retain up to one cap from each group before selecting, so children cannot consume the
  // fallback reader's script cap before the host can reserve main-document slots.
  const counts = [0, 0];
  const room = el => counts[Number(inFrame(el))] < REQ.maxRefs;
  const addRef = item => {
    if (!focusedMatch(item)) return;
    counts[Number(item.frame === true)] += 1;
    refs.push(item);
  };
  const frames = [];
  const seen = new Set();
  // Key-bearing elements are considered first within each document, named and valued as hidden.
  for (const el of everything.elements) {
    // Code and styles are not page choices. Redaction still registers their tokens above.
    if (selected || !keyHolder(el) || !visible(el) || !room(el)) continue;
    const ref = refOf(el);
    if (seen.has(ref)) continue;
    seen.add(ref);
    const item = { ref, role: actionableCandidate(el) ? roleOf(el) : "text", name: hideKeys(labelOf(el), el), tag: el.localName, node: nodeIdentity(el), priority: frameLimit(el), keyLike: true, value: KEY_TEXT };
    if (inFrame(el)) item.frame = true;
    if (!visible(el)) item.visible = false;
    addRef(item);
  }
  for (const { el, rank } of elements) {
    const ref = refOf(el);
    const alreadySeen = seen.has(ref);
    seen.add(ref);
    const role = actionableCandidate(el) || visualCandidate(el) ? roleOf(el) : "text";
    const label = select?.label(el) ?? (selected && !actionableCandidate(el) && !visualCandidate(el) && !["option", "optgroup"].includes(el.localName) ? select.text(el) : labelOf(el));
    const item = { ref, role, name: hideKeys(label, el), tag: el.localName, node: nodeIdentity(el), priority: rank,
      box: Object.fromEntries(Object.entries(globalRect(el)).map(([key, value]) => [key, Math.round(value)])) };
    item.searchText = select?.private(el) ? "" : controlSearchText(el);
    if (select?.kind(el)) item.redacted = select.kind(el);
    if (visualCandidate(el)) {
      item.coordinateKind = "region";
      item.name ||= `${el.localName} region`;
    }
    if (keyLike(label, el)) item.keyLike = true;
    if (alreadySeen) { item.keyLike = true; item.value = KEY_TEXT; }
    if (role === "iframe") {
      let src;
      try {
        const u = new URL(el.getAttribute("src") || "about:blank", location.href);
        src = u.origin === "null" ? u.href : u.origin + u.pathname;
      } catch {
        src = String(el.getAttribute("src") || "");
      }
      frames.push({ ref, name: item.name, src: clip(hideKeys(src), 200), sameOrigin: Boolean(frameDoc(el)), visible: visible(el) });
      continue;
    }
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
      else if (tag === "select" && !item.redacted) {
        const options = Array.from(el.options || []);
        item.value = clip(hideKeys(options.filter((o) => o.selected).map((o) => o.label || o.text).join(", "), el), 200);
        item.options = options.slice(0, 20).map((o) => clip(hideKeys(o.label || o.text || o.value, el), 80));
        if (keyLike(el.value, el)) item.keyLike = true;
        if (options.length > 20) item.optionsMore = options.length - 20;
      } else if (!item.redacted) {
        item.value = clip(hideKeys(el.isContentEditable ? el.innerText : el.value, el), 300);
        if (keyLike(el.value, el)) item.keyLike = true;
      }
      if (el.required) item.required = true;
      if (el.willValidate) {
        item.valid = Boolean(el.validity.valid);
        if (!item.valid && !item.redacted) item.validationMessage = clip(hideKeys(el.validationMessage, el), 300);
      }
    } else {
      const checked = el.getAttribute("aria-checked");
      if (checked !== null) item.checked = checked === "true";
      if (labelledToggle(el)) item.checked = Boolean(el.control.checked);
    }
    const selectedState = el.getAttribute("aria-selected");
    if (selectedState !== null) item.selected = selectedState === "true";
    const expanded = el.getAttribute("aria-expanded");
    if (expanded !== null) item.expanded = expanded === "true";
    const pressed = el.getAttribute("aria-pressed");
    if (pressed !== null) item.pressed = pressed === "true";
    if (tag === "a" && !item.redacted) {
      try {
        item.href = hideKeys(String(new URL(el.getAttribute("href"), location.href).href)).slice(0, 300);
      } catch {
        item.href = clip(hideKeys(el.getAttribute("href")), 300);
      }
    }
    // Inventory is independent of find, viewport scrolling and the requested ref cap.
    if (isVisible && (el.matches("button,a[href]") || ["button", "link", "checkbox", "radio", "switch"].includes(role))) {
      const doc = el.ownerDocument;
      if (!choiceGroups.has(doc)) choiceGroups.set(doc, { choices: [], total: 0 });
      const group = choiceGroups.get(doc);
      group.total += 1;
      if (group.choices.length < 200) group.choices.push({ el, item });
      for (const [root, overlay] of overlays) {
        if (!root.contains(el)) continue;
        overlay.total += 1;
        if (overlay.choices.length < 200) overlay.choices.push({ el, item });
      }
    }
    if (!alreadySeen && room(el)) addRef(item);
  }
  // Visible page text, including any dialog text.
  let text;
  try { text = REQ.scope ? scopeRoot ? select.text(scopeRoot) : "" : select ? select.text(document.body) : String(document.body?.innerText || ""); } catch { text = ""; }
  // Text that looks like a key is hidden (the element that shows it has a keyLike ref).
  text = hideKeys(text);
  let offset = Math.max(0, Math.min(Number(REQ.textOffset) || 0, text.length));
  let end = offset + REQ.maxText;
  if (offset && /[\uDC00-\uDFFF]/u.test(text[offset] || "")) offset += 1;
  if (/[\uD800-\uDBFF]/u.test(text[end - 1] || "")) end += 1;
  out.text = text.slice(offset, end);
  out.textOffset = offset;
  out.textLength = text.length;
  // Hidden text (display:none, visibility:hidden, tiny, off-canvas): only for the prompt guard.
  const hidden = [];
  let hiddenLength = 0;
  try {
    const walker = document.createTreeWalker(REQ.scope ? scopeRoot || document.createElement("div") : document.body || document.documentElement, 4);
    for (let node = walker.nextNode(); node && hiddenLength < 3000; node = walker.nextNode()) {
      const value = String(node.textContent || "").trim();
      const parent = node.parentElement;
      if (parent && select?.private(parent)) continue;
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
  } catch {
    // A page that blocks the walk only loses the hidden-text check for this observation.
  }
  out.hiddenText = hideKeys(hidden.join("\n")).slice(0, 3000);
  out.coveringFrames = [];
  out.choiceContext = [];
  out.documentContext = REQ.scope ? scopeRoot ? { frameOrigin: hideKeys(location.origin), headings: [], text: text.slice(0, 1000), complete: text.length <= 1000 && !truncated } : null : choiceContextFor(document.body || document.documentElement, select?.private);
  for (const [root, group] of overlays) {
    if (!group.total) continue;
    group.choices.sort((a, b) => a.el.compareDocumentPosition(b.el) & 2 ? 1 : -1);
    const choices = group.choices.map(({ item }) => item);
    out.coveringFrames.push({ origin: hideKeys(location.origin), choices,
      total: group.total, complete: !truncated && group.total === choices.length });
    out.choiceContext.push(choiceContextFor(root, select?.private));
  }
  for (const [doc, group] of choiceGroups) {
    group.choices.sort((a, b) => a.el.compareDocumentPosition(b.el) & 2 ? 1 : -1);
    const choices = group.choices.map(({ item }) => item);
    if (doc === document) {
      out.choiceRefs = choices;
      out.choiceCount = group.total;
    } else if (group.choices.length && frameLimit(group.choices[0].el) === 7) {
      out.coveringFrames.push({ origin: hideKeys(doc.location.origin), choices,
        total: group.total, complete: !truncated && group.total === choices.length });
      out.choiceContext.push(choiceContextFor(doc.body || doc.documentElement, select?.private));
    }
  }
  refs.sort((a, b) => b.priority - a.priority);
  let reserved = Math.min(counts[0], Math.ceil(REQ.maxRefs / 2)), remaining = REQ.maxRefs;
  out.refs = refs.filter(item => {
    const main = item.frame !== true;
    if (!remaining || (!selected && !main && remaining <= reserved)) return false;
    remaining -= 1;
    if (main) reserved = Math.max(0, reserved - 1);
    return true;
  });
  out.allRefs = Array.from(new Set([...refs.map(item => item.ref), ...seen])).slice(0, 1000);
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
