// Closed shadow roots for visual-capture admission and raw page-read checks (vw-browser `js/protected_shown.js`).
// No script world can enter a closed root once it exists, so this page-world script, first at document start in
// every frame, keeps each closed root this document's `attachShadow` creates. The admission asks with cancelable
// events on the document, the only state both worlds share: "vibework-closed-roots-ready" is always cancelled (the
// keeper runs here), and "vibework-closed-roots-shown" is cancelled while a kept root of a connected host holds a
// rendered filled field (an input with a value other than a password, hidden or choice input, a textarea, a select
// with a chosen option's text, an editable element with text), or holds more elements than are looked at. A page can
// only make the answer stricter (cancel); it cannot clear a cancellation. A document whose keeper does not answer
// (its listeners removed) is refused by the admission, never admitted.
// A closed root the HTML parser attaches (declarative shadow DOM, `<template shadowrootmode>`) never passes through
// `attachShadow`. "vibework-closed-roots-declarative" is cancelled, for good, once this document may hold one this
// keeper cannot enter: HTML with a `shadowroot` attribute given to `setHTMLUnsafe`, `parseHTMLUnsafe`, `setHTML`,
// `parseHTML` or `document.write` (of any realm of this page: a hook in another frame's realm also marks the target's
// document and every same-origin ancestor document through "vibework-closed-roots-mark"), any streaming HTML entry,
// or a document that came from a `srcdoc`, `data:` or `blob:` source holding one (unreadable source counts). A document
// from the network is checked by the engine (the private world that enters closed roots, or the admission's own
// source check); this keeper cannot read that source.
(() => {
  const READY = "vibework-closed-roots-ready";
  const SHOWN = "vibework-closed-roots-shown";
  const FIELDS = "vibework-closed-roots-fields";
  const DECLARATIVE = "vibework-closed-roots-declarative";
  const MARK = "vibework-closed-roots-mark";
  const LIMIT = 200000;
  // Captured before any page script: a page that later replaces `Reflect.apply`, `Function.prototype.call` or
  // `WeakRef` cannot see the original methods or the kept roots through them.
  const apply = Reflect.apply;
  const Weak = typeof WeakRef === "function" ? WeakRef : null;
  const attach = Element.prototype.attachShadow;
  const all = DocumentFragment.prototype.querySelectorAll;
  const rects = Element.prototype.getClientRects;
  const roots = [];
  const keep = (root) => {
    roots[roots.length] = Weak ? new Weak(root) : { deref: () => root };
  };
  if (typeof attach === "function") {
    Object.defineProperty(Element.prototype, "attachShadow", {
      configurable: true,
      enumerable: false,
      writable: true,
      value: {
        attachShadow(init) {
          const root = apply(attach, this, [init]);
          if (root && root.mode === "closed") keep(root);
          return root;
        },
      }.attachShadow,
    });
  }
  // Declarative closed roots (header). Captured now, before any page script can replace them.
  let declarative = false;
  const Ev = Event;
  const dispatch = EventTarget.prototype.dispatchEvent;
  const exec = RegExp.prototype.exec;
  const slice = String.prototype.slice;
  const tailOf = WeakMap.prototype.get;
  const keepTail = WeakMap.prototype.set;
  // Source that may hold a closed declarative root: a `shadowrootmode` not given exactly the plain value `open` (an
  // entity or another value may still make it closed) or the legacy bare `shadowroot`; other `shadowroot...`
  // attributes (`shadowrootclonable`, ...) are not modes. vw-preview `declarative_source.rs` keeps the same rule.
  const attributeName = /shadowroot/gi;
  const otherAttribute = /^shadowroot[a-ln-z]/i;
  const openMode = /^shadowrootmode[ \t\n\f\r]*=[ \t\n\f\r]*(?:"open"|'open'|open(?=[ \t\n\f\r>/]))/i;
  const risky = (value) => {
    attributeName.lastIndex = 0;
    for (let found = apply(exec, attributeName, [value]); found; found = apply(exec, attributeName, [value])) {
      const part = apply(slice, value, [found.index, found.index + 64]);
      // Another template attribute (`shadowrootclonable`, `shadowrootdelegatesfocus`, ...) is not a mode.
      if (apply(exec, otherAttribute, [part])) continue;
      if (!apply(exec, openMode, [part])) return true;
    }
    return false;
  };
  const tt = typeof trustedTypes === "object" && trustedTypes ? trustedTypes : null;
  const isHTML = tt && typeof tt.isHTML === "function" ? tt.isHTML : null;
  const trustedText = typeof TrustedHTML === "function" ? TrustedHTML.prototype.toString : null;
  const text = (html) => {
    if (typeof html === "string") return html;
    if (isHTML && trustedText && apply(isHTML, tt, [html])) return apply(trustedText, html, []);
    return null;
  };
  // Marks a document by event: its own keeper (if any) answers for it; a page can only add marks.
  const markDocument = (doc) => {
    try {
      if (doc === document) declarative = true;
      else if (doc && doc.nodeType === 9) apply(dispatch, doc, [new Ev(MARK)]);
    } catch { /* fails closed: cross-origin or detached doc cannot be marked; current doc already is */ }
  };
  const mark = (target) => {
    declarative = true;
    try { markDocument(target && target.nodeType === 9 ? target : target && target.ownerDocument); } catch { /* fails closed: unreadable target doc; declarative already set */ }
    let view = window;
    for (let depth = 0; depth < 32; depth += 1) {
      let parent;
      try { parent = view.parent; } catch { break; }
      if (!parent || parent === view) break;
      try { markDocument(parent.document); } catch { /* cross-origin ancestor blocks document read */ }
      view = parent;
    }
  };
  const checked = (target, html) => {
    const value = text(html);
    if (value === null || risky(value)) mark(target);
  };
  const wrap = (owner, name, check) => {
    const original = owner && Object.getOwnPropertyDescriptor(owner, name);
    if (!original || typeof original.value !== "function") return;
    const inner = original.value;
    Object.defineProperty(owner, name, {
      configurable: true,
      enumerable: original.enumerable,
      writable: true,
      value: {
        [name](...args) {
          check(this, args);
          return apply(inner, this, args);
        },
      }[name],
    });
  };
  for (const owner of [Element.prototype, typeof ShadowRoot === "function" ? ShadowRoot.prototype : null]) {
    wrap(owner, "setHTMLUnsafe", (target, args) => checked(target, args[0]));
    wrap(owner, "setHTML", (target, args) => checked(target, args[0]));
  }
  wrap(Document, "parseHTMLUnsafe", (target, args) => checked(document, args[0]));
  wrap(Document, "parseHTML", (target, args) => checked(document, args[0]));
  // `document.write` may split the attribute over calls: each written document keeps its last characters.
  const tails = new WeakMap();
  const written = (target, args) => {
    let all = "";
    for (let index = 0; index < args.length; index += 1) {
      const value = text(args[index]);
      if (value === null) { mark(target); return; }
      all += value;
    }
    const doc = target && target.nodeType === 9 ? target : document;
    const joined = (apply(tailOf, tails, [doc]) || "") + all;
    apply(keepTail, tails, [doc, apply(slice, joined, [-32])]);
    if (risky(joined)) mark(doc);
  };
  wrap(Document.prototype, "write", written);
  wrap(Document.prototype, "writeln", written);
  // Streaming HTML entries (not all engines have them) are taken as declarative, whatever they write.
  for (const owner of [Element.prototype, typeof ShadowRoot === "function" ? ShadowRoot.prototype : null, Document, Document.prototype]) {
    for (const name of owner ? Object.getOwnPropertyNames(owner) : []) {
      if (/^stream.*HTML/.test(name)) wrap(owner, name, (target) => mark(target));
    }
  }
  // This document's own source when it is not from the network: srcdoc, data: and blob: documents.
  try {
    const href = String(location.href);
    if (href === "about:srcdoc") {
      let source = null;
      try { source = window.frameElement && window.frameElement.getAttribute("srcdoc"); } catch { /* fails closed: unreadable srcdoc leaves source null so declarative becomes true */ }
      if (source === null || risky(source)) declarative = true;
    } else if (/^data:/i.test(href)) {
      const comma = href.indexOf(",");
      const head = href.slice(5, comma);
      const body = href.slice(comma + 1);
      let source = null;
      try { source = /;base64$/i.test(head) ? atob(decodeURIComponent(body)) : decodeURIComponent(body); } catch { /* fails closed: malformed data URL leaves source null so declarative becomes true */ }
      if (comma < 0 || source === null || risky(source)) declarative = true;
    } else if (/^blob:/i.test(href)) {
      let source = null;
      try {
        const request = new XMLHttpRequest();
        request.open("GET", href, false);
        request.send();
        source = request.status === 200 ? String(request.responseText) : null;
      } catch { /* fails closed: unreadable blob leaves source null so declarative becomes true */ }
      if (source === null || risky(source)) declarative = true;
    }
  } catch {
    declarative = true;
  }
  const unchecked = new Set(["password", "hidden", "checkbox", "radio", "button", "submit", "reset", "image", "file", "range", "color"]);
  const filled = (el) => {
    const tag = el.localName;
    if (tag === "input") return !unchecked.has(String(el.type).toLowerCase()) && String(el.value ?? "").trim() !== "";
    if (tag === "textarea") return String(el.value ?? "").trim() !== "";
    if (tag === "select") return Array.from(el.selectedOptions || [], (option) => option.text).join("").trim() !== "";
    return el.isContentEditable === true && String(el.innerText ?? "").trim() !== "";
  };
  const shown = () => {
    let seen = 0;
    for (const ref of roots) {
      const root = ref.deref();
      // A host drawn as `display: contents` has no box of its own: each field's own boxes decide.
      if (!root || !root.host || !root.host.isConnected) continue;
      const queue = [root];
      for (let r = 0; r < queue.length; r += 1) {
        for (const el of apply(all, queue[r], ["*"])) {
          if (++seen > LIMIT) return true;
          if (el.shadowRoot) queue.push(el.shadowRoot);
          if (filled(el) && apply(rects, el, []).length) return true;
        }
      }
    }
    return false;
  };
  const fields = () => {
    let seen = 0;
    for (const ref of roots) {
      const root = ref.deref();
      if (!root || !root.host || !root.host.isConnected) continue;
      const queue = [root];
      for (let r = 0; r < queue.length; r += 1) {
        for (const el of apply(all, queue[r], ["*"])) {
          if (++seen > LIMIT) return true;
          if (el.shadowRoot) queue.push(el.shadowRoot);
          if (["input", "textarea", "select"].includes(el.localName) || el.isContentEditable) return true;
        }
      }
    }
    return false;
  };
  const ready = (event) => event.preventDefault();
  const answer = (event) => {
    let result;
    try { result = shown(); } catch { result = true; }
    if (result) event.preventDefault();
  };
  const answerFields = (event) => {
    let result;
    try { result = fields(); } catch { result = true; }
    if (result) event.preventDefault();
  };
  const answerDeclarative = (event) => {
    if (declarative) event.preventDefault();
  };
  const marked = () => {
    declarative = true;
  };
  const listen = () => {
    window.addEventListener(DECLARATIVE, answerDeclarative, true);
    window.addEventListener(MARK, marked, true);
    window.addEventListener(READY, ready, true);
    window.addEventListener(SHOWN, answer, true);
    window.addEventListener(FIELDS, answerFields, true);
  };
  listen();
  // `document.open()` erases the window's listeners; the keeper answers again after it.
  const open = Document.prototype.open;
  if (typeof open === "function") {
    Object.defineProperty(Document.prototype, "open", {
      configurable: true,
      enumerable: false,
      writable: true,
      value: {
        open(...args) {
          const result = open.apply(this, args);
          if (this === document) listen();
          return result;
        },
      }.open,
    });
  }
})();
