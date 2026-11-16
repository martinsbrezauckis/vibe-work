// Whether this document shows a protected field's value: the visual-capture admission below Full access (screenshots,
// image crops, GIF frames, full-page PNG and PDF). Protected = the shared privateField rule (password, PIN, card,
// vault-filled). A field the engine draws masked (a password input, `-webkit-text-security`), a hidden or empty field
// does not count. Fields in this document, its open shadow roots and its same-origin child documents are looked at;
// over 200,000 elements counts as shown. Closed shadow roots cannot be entered from this world: each document's
// keeper (vw-preview `js/closed_roots.js`, page world, document start) answers through two cancelable events whether
// a closed root holds a rendered filled field; a document without the keeper's answer counts as shown.
// `frames` counts the child frames this document cannot enter (another origin) and `entered` says whether the parent
// document could enter this one (and so already looked at it): the host refuses the capture when fewer not-entered
// frames answered than all documents name. Evaluated in every frame; the answer is untrusted page data.
/* global editingHost, privateField, viewOf -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  __VW_COMMON__
  const roots = [document];
  let seen = 0;
  let frames = 0;
  // The keeper cancels "ready" always and "shown" while a closed root holds a filled field.
  const asked = (doc, type) => !doc.dispatchEvent(new Event(type, { cancelable: true }));
  const closedShown = (doc) => !asked(doc, "vibework-closed-roots-ready") || asked(doc, "vibework-closed-roots-shown");
  const shown = () => {
    for (let r = 0; r < roots.length; r += 1) {
      if (roots[r].nodeType === Node.DOCUMENT_NODE && closedShown(roots[r])) return true;
      for (const el of roots[r].querySelectorAll("*")) {
        if (++seen > 200000) return true;
        if (el.shadowRoot) roots.push(el.shadowRoot);
        if (el.localName === "iframe" || el.localName === "frame") {
          let child;
          try { child = el.contentDocument; } catch { child = null; }
          if (child) roots.push(child); else frames += 1;
        }
        const tag = el.localName;
        const editing = editingHost(el);
        if (!["input", "textarea", "select"].includes(tag) && !editing) continue;
        const field = privateField(el);
        if (!field || field.kind === "hidden" || tag === "input" && String(el.type).toLowerCase() === "password") continue;
        const style = viewOf(el).getComputedStyle(el);
        if (style.webkitTextSecurity && style.webkitTextSecurity !== "none") continue;
        const value = editing ? el.innerText : tag === "select"
          ? Array.from(el.selectedOptions || [], (option) => option.text).join("") : el.value;
        if (String(value ?? "").trim() && el.getClientRects().length && style.visibility === "visible") return true;
      }
    }
    return false;
  };
  let entered;
  try { entered = window.frameElement !== null; } catch { entered = false; }
  return JSON.stringify({ shown: shown(), frames, entered });
})()
