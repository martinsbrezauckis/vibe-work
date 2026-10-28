// Put a vault login into one sign-in field using ShellX's grant and expected-origin checks. Every check runs again here, in the same
// synchronous call that writes, so nothing can change between the last check and the write: the mount's own document
// (its environment), the page origin, one element with the ref, its document's origin, its sign-in role, where its
// form submits and that it submits with POST only, visible / enabled / editable. With REQ.check the script only checks (no value is in it).
// REQ: { page, ref, role: "password"|"username", origin, check?: true, value?: string }.
// Answer: { status: ok|filled|stale|ambiguous|originChanged|frameOrigin|notCredential|formOrigin|formMethod|notActionable|
//           cancelled|error, url, failed?, target?, submitsTo? }. Never the value, never a page exception text (a page
//           could throw an error that quotes what it was given).
/* global ENV, REQ, editable, enabled, findRef, hideKeys, labelOf, loginRole, roleOf, submitMethods, submitOrigins, viewOf, visible -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  __VW_COMMON__
  const result = (status, extra) => JSON.stringify({ status, url: String(location.href).slice(0, 2000), ...extra });
  if (!ENV) return result("stale");
  try {
    if (String(location.origin) !== REQ.origin) return result("originChanged");
    const found = findRef(REQ.ref);
    if (found.length === 0) return result("stale");
    if (found.length > 1) return result("ambiguous");
    const el = found[0];
    const target = { role: roleOf(el), name: hideKeys(labelOf(el)) };
    let docOrigin = "";
    try {
      docOrigin = String(el.ownerDocument.location.origin);
    } catch {
      docOrigin = "";
    }
    // A same-origin frame (about:blank and srcdoc frames inherit the page's origin) is fine; any other is not.
    if (docOrigin !== REQ.origin) return result("frameOrigin", { target });
    if (loginRole(el) !== REQ.role) return result("notCredential", { target });
    const submitsTo = submitOrigins(el);
    if (submitsTo.some((o) => o !== REQ.origin)) return result("formOrigin", { target, submitsTo });
    // A GET (or dialog) submit would put the fields into the page address the agent reads.
    if (submitMethods(el).some((m) => m !== "post")) return result("formMethod", { target });
    const failed = [];
    if (!el.isConnected) failed.push("attached");
    if (!visible(el)) failed.push("visible");
    if (!enabled(el)) failed.push("enabled");
    if (!editable(el)) failed.push("editable");
    if (failed.length > 0) return result("notActionable", { failed, target });
    if (REQ.check) return result("ok", { target });
    const View = viewOf(el);
    if (el.focus) el.focus({ preventScroll: true });
    const text = String(REQ.value);
    if (!el.dispatchEvent(new (View.InputEvent || View.Event)("beforeinput", { bubbles: true, cancelable: true, inputType: "insertReplacementText" }))) {
      return result("cancelled", { target });
    }
    const setter = Object.getOwnPropertyDescriptor(View.HTMLInputElement.prototype, "value");
    if (setter && setter.set) setter.set.call(el, text);
    else el.value = text;
    if (REQ.role === "password") {
      // Remembered: every later read reports the field as "vault"; if the page turns it into a text field (a "show
      // password" toggle), it is emptied before the next paint (a MutationObserver runs as a microtask).
      if (!ENV.vaultFilled) ENV.vaultFilled = new WeakSet();
      ENV.vaultFilled.add(el);
      const Observer = View.MutationObserver || MutationObserver;
      const guard = new Observer(() => {
        if (String(el.type || "").toLowerCase() !== "password") {
          if (setter && setter.set) setter.set.call(el, "");
          else el.value = "";
          guard.disconnect();
        }
      });
      guard.observe(el, { attributes: true, attributeFilter: ["type"] });
    }
    el.dispatchEvent(new (View.InputEvent || View.Event)("input", { bubbles: true, inputType: "insertReplacementText" }));
    el.dispatchEvent(new View.Event("change", { bubbles: true }));
    return result("filled", { target });
  } catch {
    return result("error");
  }
})()
