// Shared helpers of the agent browser's page scripts. Prepended to observe.js, act.js and probe.js after
// `const REQ = {...};`. Everything here runs synchronously in the page's own JavaScript world: the page can
// interfere with it (that only affects its own observation); every string is clipped here and again in Rust.
// Adapted from ShellX: element identity (role, identity segment, DOM path, FNV-1a
// fingerprint), DOM traversal (bounded traversal of documents, same-origin frames and open shadow
// roots, global rects across frames), actionability (attached, visible, stable, enabled, receives
// events, the covering element).
const ENV = (() => {
  try {
    return window[REQ.page] || null;
  } catch (_) {
    return null;
  }
})();
const clip = (v, n) => String(v === undefined || v === null ? "" : v).replace(/\s+/g, " ").trim().slice(0, n);
const MAX_ROOTS = 32, MAX_NODES = 12000, MAX_FRAME_DEPTH = 4, MAX_SHADOW_DEPTH = 8;
const INTERACTIVE =
  "a[href],button,input:not([type=hidden]),textarea,select,summary,iframe,frame," +
  "[role=button],[role=link],[role=checkbox],[role=radio],[role=switch],[role=tab],[role=menuitem]," +
  "[role=option],[role=combobox],[role=textbox],[role=searchbox],[role=slider],[contenteditable=''],[contenteditable=true]," +
  "[tabindex]:not([tabindex='-1']),[onclick]";
// Some site footers install a click listener on a plain span/div and expose only a pointer cursor. Give a
// small, visibly labelled target a ref so normal hit testing can decide whether to act.
const customClickable = (el) => {
  if (matches(el, INTERACTIVE) || !["a", "span", "div", "p", "li", "label"].includes(el.localName)) return false;
  if (el.closest(INTERACTIVE)) return false;
  if (!visible(el) || !enabled(el)) return false;
  const rect = el.getBoundingClientRect();
  if (rect.width > 600 || rect.height > 120) return false;
  if (viewOf(el).getComputedStyle(el).cursor !== "pointer") return false;
  if (!clip(el.innerText || el.textContent, 160)) return false;
  return !el.querySelector(INTERACTIVE);
};
const actionableCandidate = (el) => matches(el, INTERACTIVE) || customClickable(el);

const roleOf = (el) => {
  const tag = String(el.localName || "").toLowerCase();
  const explicit = el.getAttribute && el.getAttribute("role");
  if (explicit) return clip(explicit, 30);
  if (customClickable(el)) return "button";
  if (tag === "a") return "link";
  if (tag === "button" || tag === "summary") return "button";
  if (tag === "textarea") return "textbox";
  if (tag === "select") return el.multiple ? "listbox" : "combobox";
  if (tag === "iframe" || tag === "frame") return "iframe";
  if (tag === "input") {
    const type = String(el.getAttribute("type") || "text").toLowerCase();
    if (type === "checkbox") return "checkbox";
    if (type === "radio") return "radio";
    if (["button", "submit", "reset", "image"].includes(type)) return "button";
    if (type === "range") return "slider";
    if (type === "search") return "searchbox";
    if (type === "file") return "file";
    return "textbox";
  }
  if (el.isContentEditable) return "textbox";
  return tag || "element";
};

const escCss = (v) => (window.CSS && CSS.escape ? CSS.escape(String(v)) : String(v).replace(/[^a-zA-Z0-9_-]/g, "\\$&"));
const segment = (el) => {
  const tag = el.localName.toLowerCase();
  if (el.id) return `${tag}#${escCss(el.id)}`;
  const testId = el.getAttribute("data-testid") || el.getAttribute("data-test-id");
  if (testId) return `${tag}[data-testid="${clip(testId, 80)}"]`;
  const name = el.getAttribute("name");
  if (name) return `${tag}[name="${clip(name, 80)}"]`;
  const parent = el.parentElement || (el.parentNode && el.parentNode.nodeType === 11 ? el.parentNode : null);
  if (!parent) return tag;
  const same = Array.from(parent.children).filter((c) => c.localName === el.localName);
  return same.length > 1 ? `${tag}:nth-of-type(${same.indexOf(el) + 1})` : tag;
};
const domPath = (el) => {
  const parts = [];
  let cur = el;
  for (let depth = 0; cur && cur.nodeType === 1 && depth < 16; depth += 1) {
    parts.unshift(segment(cur));
    if (cur.parentElement) {
      cur = cur.parentElement;
      continue;
    }
    const root = cur.getRootNode && cur.getRootNode();
    if (root && root.host) {
      parts.unshift(">>>");
      cur = root.host;
    } else if (root && root.nodeType === 9) {
      let frame = null;
      try {
        frame = root.defaultView && root.defaultView.frameElement;
      } catch (_) {
        frame = null;
      }
      if (!frame) break;
      parts.unshift("::frame");
      cur = frame;
    } else break;
  }
  return clip(parts.join(" > "), 512);
};
const fnv = (s) => {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i += 1) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
};
// A ref = a hash of the element's identity: tag, role, type, stable attributes (or its text when it has none) and
// its DOM path. The same element re-rendered keeps its ref; a changed one gets a new ref (the old one is "stale").
const refOf = (el) => {
  const attr = (n) => clip(el.getAttribute && el.getAttribute(n), 120);
  const stable = ["data-testid", "data-test-id", "id", "name", "aria-label", "placeholder", "href"]
    .map((n) => [n, attr(n)])
    .filter(([, v]) => v)
    .map(([n, v]) => `${n}=${v}`)
    .join("~");
  const text = stable ? "" : clip(el.innerText || el.textContent, 160);
  return "e" + fnv([el.localName, roleOf(el), attr("type"), stable, text, domPath(el)].join("|"));
};
// The ref describes markup and can be shared by duplicate controls. Bind a held native answer to the actual node
// selected at preflight, even if another identical copy becomes visible before the action or final native check.
const nodeIdentity = (el) => {
  if (!ENV || !el) return "";
  if (!ENV.nodeIdentities) {
    ENV.nodeIdentities = new WeakMap();
    ENV.nextNodeIdentity = 0;
  }
  let id = ENV.nodeIdentities.get(el);
  if (!id) {
    id = ++ENV.nextNodeIdentity;
    ENV.nodeIdentities.set(el, id);
  }
  return id;
};

const frameDoc = (frame) => {
  try {
    const doc = frame.contentDocument;
    if (!doc || !doc.documentElement) return null;
    const href = String(doc.location && doc.location.href);
    if (href === "about:blank" || href === "about:srcdoc") return doc;
    return new URL(href, location.href).origin === location.origin ? doc : null;
  } catch (_) {
    return null;
  }
};
// Every element of the page, its same-origin frames and open shadow roots (bounded); `truncated` when cut.
const collect = () => {
  const elements = [];
  const queue = [{ root: document, frame: 0, shadow: 0 }];
  const seen = new WeakSet(), seenElements = new WeakSet();
  let roots = 0, truncated = false, crossOriginFrames = 0;
  const priorityLimit = Math.min(512, MAX_NODES);
  const priority = (el) => {
    if (elements.length >= priorityLimit || seenElements.has(el) || !actionableCandidate(el)) return;
    const state = visibilityState(el);
    if (!state.visible && !state.animating) return;
    seenElements.add(el);
    elements.push(el);
  };
  const mainRegions = document.querySelectorAll('main, article, [role="main"]');
  let mainScanned = 0;
  for (const region of mainRegions) {
    if (elements.length >= priorityLimit || mainScanned >= 2048) break;
    for (const el of region.querySelectorAll(INTERACTIVE)) {
      priority(el);
      if (elements.length >= priorityLimit || ++mainScanned >= 2048) break;
    }
  }
  // Footer and menu controls often follow very large article trees. Reach them before the general node cap.
  const trailingRegions = document.querySelectorAll('footer, [role="contentinfo"], [role="menu"]');
  let trailingScanned = 0;
  for (const region of trailingRegions) {
    if (elements.length >= priorityLimit || trailingScanned >= 2048) break;
    priority(region);
    for (const el of region.querySelectorAll("*")) {
      priority(el);
      if (elements.length >= priorityLimit || ++trailingScanned >= 2048) break;
    }
  }
  while (queue.length > 0 && roots < MAX_ROOTS) {
    const ctx = queue.shift();
    if (!ctx.root || seen.has(ctx.root)) continue;
    seen.add(ctx.root);
    roots += 1;
    const doc = ctx.root.nodeType === 9 ? ctx.root : ctx.root.ownerDocument;
    const walker = doc.createTreeWalker(ctx.root, 1);
    for (let el = walker.nextNode(); el; el = walker.nextNode()) {
      if (elements.length >= MAX_NODES) {
        truncated = true;
        break;
      }
      if (!seenElements.has(el)) {
        seenElements.add(el);
        elements.push(el);
      }
      if (el.shadowRoot) {
        if (ctx.shadow < MAX_SHADOW_DEPTH) queue.push({ root: el.shadowRoot, frame: ctx.frame, shadow: ctx.shadow + 1 });
        else truncated = true;
      }
      const tag = el.localName;
      if (tag === "iframe" || tag === "frame") {
        const inner = frameDoc(el);
        if (!inner) crossOriginFrames += 1;
        else if (ctx.frame < MAX_FRAME_DEPTH) queue.push({ root: inner, frame: ctx.frame + 1, shadow: ctx.shadow });
        else truncated = true;
      }
    }
  }
  if (queue.length > 0) truncated = true;
  return { elements, truncated, crossOriginFrames };
};
const matches = (el, selector) => {
  try {
    return el.matches(selector);
  } catch (_) {
    return false;
  }
};
const candidates = () => {
  const all = collect();
  return { ...all, elements: all.elements.filter(actionableCandidate) };
};

const viewOf = (el) => (el.ownerDocument && el.ownerDocument.defaultView) || window;
const inFrame = (el) => el.ownerDocument !== document;
// Walk the rendered ancestry, including slotted content, open/closed shadow hosts and same-origin frame elements.
const composedParent = (el) => {
  if (el.assignedSlot) return el.assignedSlot;
  if (el.parentElement) return el.parentElement;
  const root = el.getRootNode && el.getRootNode();
  if (root?.host) return root.host;
  try { return root?.defaultView?.frameElement || null; } catch (_) { return null; }
};
const globalRect = (el) => {
  const r = el.getBoundingClientRect();
  let left = r.left, top = r.top, width = r.width, height = r.height;
  let doc = el.ownerDocument;
  for (let depth = 0; doc && depth < MAX_FRAME_DEPTH; depth += 1) {
    let frame = null;
    try {
      frame = doc.defaultView && doc.defaultView.frameElement;
    } catch (_) {
      frame = null;
    }
    if (!frame) break;
    const fr = frame.getBoundingClientRect();
    const scaleX = fr.width / frame.offsetWidth, scaleY = fr.height / frame.offsetHeight;
    left = fr.left + (frame.clientLeft + left) * scaleX;
    top = fr.top + (frame.clientTop + top) * scaleY;
    width *= scaleX;
    height *= scaleY;
    doc = frame.ownerDocument;
  }
  return { x: left, y: top, width, height };
};
const visibilityState = (el) => {
  const r = el.getBoundingClientRect();
  const hasBox = r.width > 0 && r.height > 0;
  let opacity = 1, animating = false;
  for (let cur = el, depth = 0; cur && depth < 256; cur = composedParent(cur), depth += 1) {
    const style = viewOf(cur).getComputedStyle(cur);
    // Computed visibility on the target includes inheritance, while a descendant may explicitly restore
    // visibility inside a hidden ancestor. A frame's own visibility is a separate document boundary.
    const visibilityBoundary = cur === el || cur.localName === "iframe" || cur.localName === "frame";
    if (style.display === "none" || cur.inert || (visibilityBoundary
      && (style.visibility === "hidden" || style.visibility === "collapse"))) {
      return { visible: false, animating: false };
    }
    opacity *= Number(style.opacity);
    if (cur.getAnimations && !animating) {
      try {
        animating = cur.getAnimations({ subtree: false }).some((animation) => animation.playState === "running"
          && animation.effect?.getKeyframes?.().some((frame) =>
            ["opacity", "transform", "translate", "scale", "left", "top"].some((property) => property in frame)));
      } catch (_) { /* An unavailable animation API cannot turn a hidden target into a click. */ }
    }
  }
  return { visible: hasBox && opacity > 0, animating };
};
const visible = (el) => visibilityState(el).visible;
const enabled = (el) => {
  if (el.disabled || (el.getAttribute && el.getAttribute("aria-disabled") === "true")) return false;
  for (let cur = el, depth = 0; cur && depth < 256; cur = composedParent(cur), depth += 1) {
    if (cur.inert || matches(cur, "fieldset[disabled]")) return false;
  }
  return true;
};
const editable = (el) => {
  if (el.isContentEditable) return true;
  const tag = el.localName;
  if (tag === "textarea") return enabled(el) && !el.readOnly;
  if (tag !== "input") return false;
  const type = String(el.getAttribute("type") || "text").toLowerCase();
  return !["button", "submit", "reset", "checkbox", "radio", "file", "hidden", "image", "range", "color"].includes(type)
    && enabled(el) && !el.readOnly;
};

// Words that mark one-time-code and payment-card fields (by name, id, label, placeholder, test id).
const OTP_META = /\b(otp|totp|one[-_ ]?time|passcode|verification[-_ ]?code|2fa|mfa|sms[-_ ]?code|auth(entication)?[-_ ]?code)\b/;
const CARD_META = /(card[-_ ]?(number|no|num)|cc[-_ ]?(num|number)|ccnum|\bcvc\b|\bcvv\b|\bcsc\b|security[-_ ]?code|expir)/;
// The kind of secret a field holds (its value never leaves the page): "password", "otp" or "card". These, and values the vault filled, are the ONLY field values the agent never reads;
// every other value, also text the user typed, is readable.
const secretKind = (el) => {
  const tag = el.localName;
  if (tag !== "input" && tag !== "textarea" && !el.isContentEditable) return null;
  const type = String(el.getAttribute("type") || "").toLowerCase();
  const auto = String(el.getAttribute("autocomplete") || "").toLowerCase();
  if (type === "password" || /\b(current|new)-password\b/.test(auto)) return "password";
  // A field the page masks with CSS (`-webkit-text-security`) is a password field shown as dots.
  try {
    const masked = viewOf(el).getComputedStyle(el).webkitTextSecurity;
    if (masked && masked !== "none") return "password";
  } catch (_) {
    // No style (a detached element): judged by its attributes only.
  }
  if (/\bone-time-code\b/.test(auto)) return "otp";
  if (/\bcc-(number|csc|exp|exp-month|exp-year)\b/.test(auto)) return "card";
  const meta = ["name", "id", "aria-label", "placeholder", "data-testid"]
    .map((n) => String(el.getAttribute(n) || ""))
    .join(" ")
    .toLowerCase();
  if (OTP_META.test(meta)) return "otp";
  if (CARD_META.test(meta)) return "card";
  if (/\bpass(word|wd)?\b|\bpin\b/.test(meta)) return "password";
  return null;
};
// The role a field plays in a sign-in form, from its LIVE state:
// "password" (an input whose current type is password and that is not a one-time-code or card field), "username" (a
// text or email input with autocomplete "username", or the email / user-name field of a form that has a password
// field), else null. Only these two roles may receive a vault login; one-time codes and cards stay the user's.
const fieldMeta = (el) => ["name", "id", "aria-label", "placeholder", "data-testid"]
  .map((n) => String(el.getAttribute(n) || ""))
  .join(" ")
  .toLowerCase();
const loginRole = (el) => {
  if (!el || el.localName !== "input") return null;
  const type = String(el.type || "").toLowerCase();
  const auto = String(el.getAttribute("autocomplete") || "").toLowerCase();
  const meta = fieldMeta(el);
  if (/\bone-time-code\b|\bcc-/.test(auto) || OTP_META.test(meta) || CARD_META.test(meta)) return null;
  if (type === "password") return "password";
  if (!["text", "email", "tel"].includes(type)) return null;
  if (secretKind(el)) return null;
  if (/\busername\b/.test(auto)) return "username";
  // Otherwise only the user-name / email field of a form that has a password field.
  const form = el.form;
  if (!form) return null;
  const doc = el.ownerDocument;
  const hasPassword = Array.from(doc.querySelectorAll("input")).some(
    (f) => f.form === form && String(f.type || "").toLowerCase() === "password",
  );
  if (!hasPassword) return null;
  if (type === "email" || /\bemail\b/.test(auto)) return "username";
  return /\b(user(name)?|login|e-?mail|account|identifier)\b/.test(`${meta} ${labelOf(el).toLowerCase()}`) ? "username" : null;
};
// Every origin the field's form can submit to: its action (an empty action is the document's own
// address), and each submit button's formaction, resolved against the document's base address as the browser does.
// Attributes are read with getAttribute (a field named "action" would shadow form.action). A field outside any form
// submits nowhere by itself: []. An address that does not resolve: "invalid".
const submitOrigins = (el) => {
  const form = el.form;
  if (!form) return [];
  const doc = el.ownerDocument;
  const resolve = (raw) => {
    const text = String(raw === null || raw === undefined ? "" : raw).trim();
    try {
      return text === "" ? new URL(doc.URL).origin : new URL(text, doc.baseURI).origin;
    } catch (_) {
      return "invalid";
    }
  };
  const out = new Set([resolve(form.getAttribute("action"))]);
  for (const b of doc.querySelectorAll("button[formaction],input[formaction]")) {
    if (b.form !== form) continue;
    const type = String(b.getAttribute("type") || (b.localName === "button" ? "submit" : "")).toLowerCase();
    if (type === "submit" || type === "image") out.add(resolve(b.getAttribute("formaction")));
  }
  return Array.from(out).map((o) => clip(o, 200));
};
// Every method the field's form can submit with: its `method` (missing or unknown is
// "get"), and each submit button's `formmethod`. A GET submit puts the fields, the password too, into the page address,
// which the agent reads; a fill needs every one to be "post". A field outside any form: [].
const submitMethods = (el) => {
  const form = el.form;
  if (!form) return [];
  const doc = el.ownerDocument;
  const norm = (raw) => {
    const m = String(raw === null || raw === undefined ? "" : raw).trim().toLowerCase();
    return m === "post" || m === "dialog" ? m : "get";
  };
  const out = new Set([norm(form.getAttribute("method"))]);
  for (const b of doc.querySelectorAll("button[formmethod],input[formmethod]")) {
    if (b.form !== form) continue;
    const type = String(b.getAttribute("type") || (b.localName === "button" ? "submit" : "")).toLowerCase();
    if (type === "submit" || type === "image") out.add(norm(b.getAttribute("formmethod")));
  }
  return Array.from(out);
};
// A field Vibe Work filled with a vault password (the page environment remembers it); every read reports
// it as "vault" whatever the page made of it since.
const vaultFilled = (el) => Boolean(ENV && ENV.vaultFilled && ENV.vaultFilled.has(el));
const labelOf = (el) => {
  const attr = (n) => el.getAttribute && el.getAttribute(n);
  if (attr("aria-label")) return clip(attr("aria-label"), 160);
  const by = attr("aria-labelledby");
  if (by) {
    const root = el.getRootNode ? el.getRootNode() : document;
    const text = by
      .split(/\s+/)
      .map((id) => {
        const target = root.getElementById ? root.getElementById(id) : document.getElementById(id);
        return target ? target.innerText || target.textContent : "";
      })
      .join(" ");
    if (text.trim()) return clip(text, 160);
  }
  if (el.labels && el.labels.length) return clip(Array.from(el.labels).map((l) => l.innerText || l.textContent).join(" "), 160);
  const tag = el.localName;
  if (tag === "input" || tag === "textarea" || tag === "select") {
    return clip(attr("title") || attr("placeholder") || attr("name") || "", 160);
  }
  if (tag === "iframe" || tag === "frame") return clip(attr("title") || attr("name") || "", 160);
  if (tag === "img") return clip(attr("alt") || attr("title") || "", 160);
  const text = clip(el.innerText || el.textContent, 160);
  if (text) return text;
  const img = el.querySelector && el.querySelector("img[alt]");
  return clip(attr("title") || (img && img.getAttribute("alt")) || "", 160);
};
// Finds the element of a ref among the interactive elements (the same traversal as the observation).
const findRef = (ref) => {
  const found = candidates().elements.filter((el) => refOf(el) === ref);
  return found;
};
// A duplicated ref can denote repeated copies of the same control. It is usable only when exactly one copy is
// rendered; the requested ref is never substituted with a different ref or an arbitrary first match.
const actionRef = (ref) => {
  const found = findRef(ref);
  if (found.length <= 1) return { found, target: found[0] || null };
  const identity = (el) => [domPath(el), roleOf(el), labelOf(el), el.localName,
    el.getAttribute("type"), el.getAttribute("name"), el.getAttribute("href")].join("|");
  if (!found.every((el) => identity(el) === identity(found[0]))) return { found, target: null };
  const shown = found.filter(visible);
  const onScreen = shown.filter((el) => {
    const r = globalRect(el);
    return r.x < window.innerWidth && r.y < window.innerHeight && r.x + r.width > 0 && r.y + r.height > 0;
  });
  const entering = shown.length ? [] : found.filter((el) => visibilityState(el).animating);
  return { found, target: onScreen.length === 1 ? onScreen[0]
    : shown.length === 1 ? shown[0] : entering.length === 1 ? entering[0] : null };
};
const describe = (el) => (el ? { role: roleOf(el), name: labelOf(el) } : null);

// Text that looks like an API key or token: hidden from every read the agent gets. Known prefixes,
// JWTs and any 32+ character run of letters, digits, `_` and `-` that mixes upper case, lower case and digits are
// hidden; unlabelled hex strings and UUIDs stay visible. Nearby credential labels also hide shorter high-entropy
// tokens, including letters-only issued keys. A heuristic for the user's convenience: stored keys are also
// withheld by value (vw-secrets).
// No lookbehind (older WebKit rejects it, and one syntax error would break every read): the generic pattern keeps
// the character before the run in group 1.
const KEY_KNOWN =
  "\\b(?:sk|pk|rk)[-_](?:[A-Za-z]{2,6}[-_])?[A-Za-z0-9_-]{16,}" +
  "|\\bgh[pousr]_[A-Za-z0-9]{20,}|\\bgithub_pat_[A-Za-z0-9_]{20,}|\\bxox[abprs]-[A-Za-z0-9-]{10,}" +
  "|\\bAKIA[0-9A-Z]{16}\\b|\\bAIza[0-9A-Za-z_-]{30,}|\\bglpat-[A-Za-z0-9_-]{16,}|\\bhf_[A-Za-z0-9]{20,}" +
  "|\\beyJ[A-Za-z0-9_-]{8,}\\.[A-Za-z0-9_-]{8,}\\.[A-Za-z0-9_-]{8,}";
const KEY_GENERIC =
  "(^|[^A-Za-z0-9_-])(?=[A-Za-z0-9_-]*[a-z])(?=[A-Za-z0-9_-]*[A-Z])(?=[A-Za-z0-9_-]*[0-9])[A-Za-z0-9_-]{32,}(?![A-Za-z0-9_-])";
const KEY_TEXT = "[looks like a key: hidden]";
const KEY_LABEL = /\b(?:api[\s_-]+key|access[\s_-]+key|client[\s_-]+secret|key|token|secret)s?\b/i;
const KEY_LABELLED = "(^|[^A-Za-z0-9_+/-])[A-Za-z0-9_-]{16,}(?![A-Za-z0-9_+/=-])";
const labelledValues = new Set();
const keyOwnText = (el) => Array.from((el && el.childNodes) || [])
  .filter((node) => node.nodeType === 3).map((node) => node.nodeValue).join(" ");
// Look only at the value's own element, its immediate wrapper and explicit label relations. A heading elsewhere
// on the page must not turn unrelated identifiers into credentials.
const keyLabelled = (el) => {
  if (!el) return false;
  const parent = el.parentElement;
  const nearby = [keyOwnText(el), labelOf(el), el.getAttribute("aria-label"), keyOwnText(parent),
    parent && parent.getAttribute("aria-label")];
  for (const node of [el.previousElementSibling, el.nextElementSibling, parent && parent.previousElementSibling]) {
    if (node && (node.textContent || "").length <= 240) nearby.push(node.textContent);
  }
  if (el.labels) nearby.push(...Array.from(el.labels).map((label) => label.textContent));
  const root = el.getRootNode();
  for (const id of (el.getAttribute("aria-labelledby") || "").split(/\s+/).filter(Boolean)) {
    const label = root.getElementById && root.getElementById(id);
    if (label) nearby.push(label.textContent);
  }
  const cell = el.closest("td,[role=cell]");
  const row = cell && cell.closest("tr,[role=row]");
  if (row) nearby.push(...Array.from(row.querySelectorAll("th,[role=rowheader]")).map((header) => header.textContent));
  return nearby.some((text) => KEY_LABEL.test(text || ""));
};
const imageName = (text, end) => /^\.(?:png|jpe?g|gif|webp|svg|avif)\b/i.test(text.slice(end));
const ordinaryIdentifier = (token) => /^[a-f0-9]+$/i.test(token) ||
  /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i.test(token);
// At least three bits per character rejects repeated placeholders; case and digit diversity are not required for
// issued keys. Filename suffixes distinguish common image identifiers from bare credential values.
const credentialToken = (token) => {
  if (!/[A-Za-z]/.test(token)) return false;
  // Letters-only values are easy to confuse with prose or identifiers. Require an all-uppercase issued-key shape.
  if (!/[0-9_-]/.test(token) && !/^[A-Z]{16,}$/.test(token)) return false;
  const counts = new Map();
  for (const char of token) counts.set(char, (counts.get(char) || 0) + 1);
  let entropy = 0;
  for (const count of counts.values()) {
    const probability = count / token.length;
    entropy -= probability * Math.log2(probability);
  }
  return entropy >= 3;
};
const keyTokens = (s, el) => {
  const text = String(s === undefined || s === null ? "" : s);
  if (!text) return [];
  const values = Array.from(text.matchAll(new RegExp(KEY_KNOWN, "g"))).map((match) => match[0]);
  for (const pattern of [KEY_GENERIC, ...(keyLabelled(el) ? [KEY_LABELLED] : [])]) {
    for (const match of text.matchAll(new RegExp(pattern, "g"))) {
      const token = match[0].slice(match[1].length);
      if (pattern === KEY_GENERIC && ordinaryIdentifier(token)) continue;
      if (!imageName(text, match.index + match[0].length) && (pattern === KEY_GENERIC || credentialToken(token))) values.push(token);
    }
  }
  return Array.from(new Set(values));
};
const keyLike = (s, el) => keyTokens(s, el).length > 0;
const hideKeys = (s, el) => {
  let text = String(s === undefined || s === null ? "" : s);
  const values = Array.from(new Set([...keyTokens(text, el), ...labelledValues])).sort((a, b) => b.length - a.length);
  for (const value of values) text = text.split(value).join(KEY_TEXT);
  return text;
};
// Attributes a copy button keeps its text in.
const COPY_ATTRS = ["value", "data-clipboard-text", "data-copy", "data-copy-text", "data-value"];
// Register before clipping, pagination or the ref cap, so a labelled value's other copies stay hidden too.
const rememberLabelledKeys = (el) => {
  const values = [keyOwnText(el), el.value, ...COPY_ATTRS.map((attr) => el.getAttribute(attr))];
  for (const value of values) for (const token of keyTokens(value, el)) labelledValues.add(token);
};
// True for a non-interactive element whose own text looks like a key (a <code>, a <span>), or any element whose copy
// attribute holds one (a copy button): such elements get a ref so the agent can name them.
const keyHolder = (el) => {
  if (!el || el.nodeType !== 1) return false;
  for (const a of COPY_ATTRS) {
    const v = el.getAttribute && el.getAttribute(a);
    if (v && keyLike(v, el) && !(el.localName === "input" || el.localName === "textarea")) return true;
  }
  if (matches(el, INTERACTIVE)) return false;
  for (const node of el.childNodes || []) {
    if (node.nodeType === 3 && keyLike(node.nodeValue, el)) return true;
  }
  return false;
};
// One authorization document fingerprint. This same function runs before the dialog and immediately before an
// automated action; a changed URL, title, client, or scope list invalidates the earlier answer.
const consentSnapshot = (reference = "") => {
  const url = String(location.href);
  const parsed = new URL(url);
  // The page environment is created afresh for each document. Bind approval to that instance too, so a reload of
  // identical content cannot inherit an answer given to the previous document.
  if (ENV && !ENV.consentDocumentId) {
    ENV.consentDocumentId = globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random()}`;
  }
  // Authorization metadata is read only from hidden fields. A hostile page can name a password field "scope";
  // its value must never enter a dialog or an action request.
  const fields = Array.from(document.querySelectorAll('input[type="hidden"][name="scope"], input[type="hidden"][name="scopes"], input[type="hidden"][name="client_id"], input[type="hidden"][name="clientId"], input[type="hidden"][name="access_type"]'))
    .filter((f) => !secretKind(f) && !vaultFilled(f) && !f.hasAttribute("data-secret"));
  const scopes = [...parsed.searchParams.getAll("scope"), ...parsed.searchParams.getAll("scopes"), ...fields.filter((f) => /^(scope|scopes)$/i.test(f.name)).map((f) => String(f.value || ""))];
  const clients = [...parsed.searchParams.getAll("client_id"), ...parsed.searchParams.getAll("clientId"), ...fields.filter((f) => /^(client_id|clientId)$/i.test(f.name)).map((f) => String(f.value || ""))];
  const accessTypes = [...parsed.searchParams.getAll("access_type"), ...fields.filter((f) => f.name === "access_type").map((f) => String(f.value || ""))];
  if (accessTypes.some((value) => value.toLowerCase() === "offline")) scopes.push("offline_access");
  const oversized = scopes.length > 40 || scopes.some((s) => s.length > 500) || clients.length > 10 || clients.some((s) => s.length > 300) || url.length > 4000;
  if (oversized) scopes.splice(0, scopes.length, "permission list too long to verify");
  const uniqueClients = [...new Set(clients.filter(Boolean))];
  const client = uniqueClients.length === 1 ? uniqueClients[0] : "";
  const title = String(document.title || "").slice(0, 200);
  const heading = String(document.querySelector("h1")?.textContent || "").trim().slice(0, 200);
  const chooser = /^(choose|pick|select) (an? )?account\b/i.test(heading) || /^(choose|pick|select) (an? )?account\b/i.test(title);
  const element = reference ? actionRef(reference).target : null;
  const targetMarkup = element ? String(element.outerHTML || "") : "";
  const targetText = element ? String(element.textContent || "").trim().slice(0, 200) : "";
  const targetLabel = element ? String(element.getAttribute("aria-label") || "").trim().slice(0, 200) : "";
  const grantLabel = /\b(allow|approve|authorize|grant)\b/i.test(`${targetText} ${targetLabel}`);
  const accountOption = Boolean(element && !grantLabel && (element.closest('[data-identifier], [data-email], [data-account-option]')
    || /\b[^\s@]+@[^\s@]+\.[^\s@]+\b/.test(targetText)
    || /^use another account$/i.test(targetText)));
  const targetAction = element ? String(element.getAttribute("formaction") || element.form?.action || element.href || "") : "";
  const snapshot = JSON.stringify({ documentId: ENV?.consentDocumentId || "", nodeId: nodeIdentity(element), url, title, client, scopes, chooser,
    targetMarkupLength: targetMarkup.length, targetMarkupHash: fnv(targetMarkup), targetText, targetLabel,
    targetActionLength: targetAction.length, targetActionHash: fnv(targetAction) });
  return { url, title, client, scopes, accountChooser: chooser && accountOption, oversized, snapshot };
};
