// Closed shadow roots for the visual-capture admission below Full access (vw-browser `js/protected_shown.js`).
// No script world can enter a closed root once it exists, so this page-world script, first at document start in
// every frame, keeps each closed root this document's `attachShadow` creates. The admission asks with two cancelable
// events on the document, the only state both worlds share: "vibework-closed-roots-ready" is always cancelled (the
// keeper runs here), and "vibework-closed-roots-shown" is cancelled while a kept root of a connected host holds a
// rendered filled field (an input with a value other than a password, hidden or choice input, a textarea, a select
// with a chosen option's text, an editable element with text), or holds more elements than are looked at. A page can
// only make the answer stricter (cancel); it cannot clear a cancellation. A closed root the HTML parser attaches
// (declarative shadow DOM) never passes through `attachShadow` and is not seen here. A document whose keeper does not
// answer (its listeners removed) is refused by the admission, never admitted.
(() => {
  const READY = "vibework-closed-roots-ready";
  const SHOWN = "vibework-closed-roots-shown";
  const LIMIT = 200000;
  const attach = Element.prototype.attachShadow;
  const all = DocumentFragment.prototype.querySelectorAll;
  const rects = Element.prototype.getClientRects;
  const roots = [];
  const keep = (root) => {
    roots.push(typeof WeakRef === "function" ? new WeakRef(root) : { deref: () => root });
  };
  if (typeof attach === "function") {
    Object.defineProperty(Element.prototype, "attachShadow", {
      configurable: true,
      enumerable: false,
      writable: true,
      value: {
        attachShadow(init) {
          const root = attach.call(this, init);
          if (root && root.mode === "closed") keep(root);
          return root;
        },
      }.attachShadow,
    });
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
        for (const el of all.call(queue[r], "*")) {
          if (++seen > LIMIT) return true;
          if (el.shadowRoot) queue.push(el.shadowRoot);
          if (filled(el) && rects.call(el).length) return true;
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
  const listen = () => {
    window.addEventListener(READY, ready, true);
    window.addEventListener(SHOWN, answer, true);
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
