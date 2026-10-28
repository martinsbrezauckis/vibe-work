// One agent action on a ref of the latest observation: find the element by its ref, check it is
// actionable, then act like a user would (pointer and key events, the native value setter so frameworks see the
// change). Synthetic events: the page environment never counts them as the user's typing.
// REQ: { page, action: click|type|select|check|press|hover|scroll, ref?, text?, value?, checked?, key?, submit?,
//        direction?, amount? }.
// Answer: { status: done|stale|ambiguous|secret|notActionable|noOption|error, ... }. Synchronous.
// Adapted from ShellX control script (native value setter, beforeinput/input/
// change, contenteditable insertText) and actionability checks; changed: Enter submits the form the way a
// browser does for a real key press, select uses the native setter.
/* global ENV, MAX_FRAME_DEPTH, MAX_SHADOW_DEPTH, REQ, actionRef, candidates, clip, composedParent, consentSnapshot, describe, editable, enabled, globalRect, inFrame, labelOf, nativeClickPoint, refOf, roleOf, secretKind, viewOf, visibilityState, visible -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  __VW_COMMON__;
  __VW_NATIVE_CLICK__;
  const result = (status, extra) =>
    JSON.stringify({
      status,
      url: String(location.href).slice(0, 2000),
      ...extra,
    });
  // Bound to the mount the action was authorized for: another mount's document has no environment
  // under this accessor, so nothing is done there ("stale": read again).
  if (!ENV) return result("stale");
  const authPage = (() => {
    const u = new URL(location.href);
    if (
      u.protocol === "http:" &&
      REQ.testConsentOrigin &&
      u.origin === REQ.testConsentOrigin &&
      u.pathname.startsWith("/o/oauth2/")
    )
      return true;
    if (u.protocol !== "https:" || u.port || u.username || u.password)
      return false;
    return (
      (u.hostname === "accounts.google.com" &&
        (u.pathname.startsWith("/o/oauth2/") ||
          u.pathname === "/signin/oauth/consent")) ||
      (u.hostname === "github.com" &&
        u.pathname === "/login/oauth/authorize") ||
      (u.hostname === "appleid.apple.com" &&
        u.pathname === "/auth/authorize") ||
      (u.hostname === "login.microsoftonline.com" &&
        /^\/[^/]+\/oauth2\/v2\.0\/authorize$/.test(u.pathname))
    );
  })();
  // The host approved one document and one ref. Every mutating route, including Enter and form submit, passes
  // through here before any event reaches the page. A changed document must be observed and classified again.
  if (
    (authPage || REQ.consent) &&
    !REQ.postconditionOnly &&
    !REQ.nativeUser &&
    !["hover", "scroll"].includes(REQ.action)
  ) {
    if (
      !REQ.consent ||
      REQ.consent.snapshot !== consentSnapshot(REQ.ref || "").snapshot ||
      REQ.consent.ref !== (REQ.ref || "")
    ) {
      return result("consentChanged");
    }
    if (
      REQ.action === "press" &&
      !REQ.ref &&
      ["Enter", " ", "Space"].includes(REQ.key)
    )
      return result("consentChanged");
  }
  const NEEDS = {
    click: ["visible", "stable", "enabled", "receivesEvents"],
    check: ["visible", "stable", "enabled", "receivesEvents"],
    type: ["visible", "stable", "enabled", "editable"],
    select: ["visible", "stable", "enabled"],
    hover: ["visible", "stable", "receivesEvents"],
    press: ["visible", "enabled"],
    scroll: [],
  };
  const deepHit = (doc, x, y) => {
    let hit = doc.elementFromPoint(x, y);
    for (let i = 0; hit && hit.shadowRoot && i < MAX_SHADOW_DEPTH; i += 1) {
      const inner = hit.shadowRoot.elementFromPoint(x, y);
      if (!inner || inner === hit) break;
      hit = inner;
    }
    return hit;
  };
  const contains = (outer, node) => {
    for (let cur = node, i = 0; cur && i < 256; cur = composedParent(cur), i += 1) {
      if (cur === outer) return true;
    }
    return false;
  };
  const hitAt = (el, x, y) => {
    let doc = el.ownerDocument, hit = deepHit(doc, x, y);
    if (!hit || !contains(el, hit)) return { valid: false, hit };
    for (let depth = 0; depth < MAX_FRAME_DEPTH; depth += 1) {
      const frame = doc.defaultView?.frameElement;
      if (!frame) return { valid: true, hit };
      const r = frame.getBoundingClientRect();
      x = r.left + (frame.clientLeft + x) * (r.width / frame.offsetWidth);
      y = r.top + (frame.clientTop + y) * (r.height / frame.offsetHeight);
      doc = frame.ownerDocument;
      if (x < 0 || y < 0 || x >= doc.defaultView.innerWidth || y >= doc.defaultView.innerHeight) return { valid: false, hit: null };
      hit = deepHit(doc, x, y);
      if (!hit || !contains(frame, hit)) return { valid: false, hit };
    }
    return { valid: false, hit: null };
  };
  const hitPoints = (el) => {
    const r = el.getBoundingClientRect(), view = viewOf(el);
    const left = Math.max(0, r.left), top = Math.max(0, r.top);
    const right = Math.min(view.innerWidth, r.right), bottom = Math.min(view.innerHeight, r.bottom);
    if (right <= left || bottom <= top) return [];
    const x = (fraction) => left + (right - left) * fraction;
    const y = (fraction) => top + (bottom - top) * fraction;
    return [[0.5, 0.5], [0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75],
      [0.1, 0.5], [0.9, 0.5], [0.5, 0.1], [0.5, 0.9]]
      .map(([fx, fy]) => ({ x: x(fx), y: y(fy) }));
  };
  // Actionability (ShellX checks): attached, visible, stable (same box over three samples spanning >= 120 ms), enabled,
  // editable, receives events (a point inside the viewport-intersected box hits the element or its child).
  const checks = (el, action) => {
    const failed = [];
    let covering = null, hitPoint = null;
    if (!el.isConnected) return { failed: ["attached"], covering, hitPoint };
    const visibility = visibilityState(el), isVisible = visibility.visible;
    const needs = NEEDS[action] || [];
    if (needs.includes("visible") && !isVisible) failed.push("visible");
    if (needs.includes("enabled") && !enabled(el)) failed.push("enabled");
    if (needs.includes("editable") && !editable(el)) failed.push("editable");
    if (isVisible && needs.includes("stable")) {
      const r = globalRect(el);
      const box = [r.x, r.y, r.width, r.height];
      const store = ENV ? ENV.stability : null;
      const prev = store ? store.get(el) : null;
      const now = performance.now();
      const same =
        prev && prev.box.every((v, i) => Math.abs(v - box[i]) <= 0.5);
      const since = same ? prev.since : now;
      const samples = same ? prev.samples + 1 : 1;
      if (store) store.set(el, { box, since, samples });
      // Three equal samples, not two (two samples taken symmetrically around the turning point of
      // an `alternate` animation showed the same box; a third sample in between differs).
      if (!(samples >= 3 && now - since >= 120)) failed.push("stable");
    }
    if (isVisible && needs.includes("receivesEvents")) {
      let blocked = null;
      for (const point of hitPoints(el)) {
        const hit = hitAt(el, point.x, point.y);
        if (hit.valid) { hitPoint = point; break; }
        blocked ||= hit.hit;
      }
      if (!hitPoint) {
        failed.push("receivesEvents");
        if (blocked) covering = { ref: refOf(blocked), ...describe(blocked), tag: blocked.localName };
      }
    }
    const transient = failed.length > 0 && failed.every((reason) =>
      ["visible", "stable", "receivesEvents"].includes(reason));
    return { failed, covering, hitPoint, retryable: visibility.animating && transient };
  };
  const point = (el, hitPoint = null) => {
    const r = el.getBoundingClientRect();
    return { clientX: hitPoint?.x ?? r.left + r.width / 2, clientY: hitPoint?.y ?? r.top + r.height / 2,
      bubbles: true, cancelable: true, composed: true, view: viewOf(el) };
  };
  const fire = (el, type, Kind, init) => {
    const View = viewOf(el);
    const Ctor = View[Kind] || View.Event;
    return el.dispatchEvent(new Ctor(type, init));
  };
  const clickLike = (el, hitPoint) => {
    const p = point(el, hitPoint);
    fire(el, "pointerover", "PointerEvent", p);
    fire(el, "mouseover", "MouseEvent", p);
    fire(el, "pointerdown", "PointerEvent", { ...p, button: 0, buttons: 1 });
    fire(el, "mousedown", "MouseEvent", { ...p, button: 0, buttons: 1 });
    if (el.focus) el.focus({ preventScroll: true });
    fire(el, "pointerup", "PointerEvent", { ...p, button: 0 });
    fire(el, "mouseup", "MouseEvent", { ...p, button: 0 });
    el.click();
  };
  const setValue = (el, value) => {
    const View = viewOf(el);
    const proto =
      el.localName === "textarea"
        ? View.HTMLTextAreaElement.prototype
        : el.localName === "select"
          ? View.HTMLSelectElement.prototype
          : View.HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, "value");
    if (setter && setter.set) setter.set.call(el, value);
    else el.value = value;
  };
  const checkedState = (el) => el.localName === "input"
    ? Boolean(el.checked) : el.getAttribute("aria-checked") === "true";
  const requestedOption = (el) => {
    const want = String(REQ.value || ""), options = Array.from(el.options || []);
    return options.find((o) => o.value === want) ||
      options.find((o) => clip(o.label || o.text, 200) === clip(want, 200)) ||
      options.find((o) => clip(o.label || o.text, 200).toLowerCase() === clip(want, 200).toLowerCase());
  };
  const postcondition = (el, target) => {
    if (REQ.action === "check") {
      const checked = checkedState(el);
      return result(checked === (REQ.checked !== false) ? "done" : "unmet", { target, checked });
    }
    if (REQ.action === "select") {
      const option = requestedOption(el), selected = el.options?.[el.selectedIndex];
      return result(option && selected === option ? "done" : "unmet", {
        target, value: clip(selected?.label || selected?.text, 200),
      });
    }
    if (REQ.action === "type") {
      const actual = String(el.isContentEditable ? el.innerText : el.value);
      return result(actual === String(REQ.text || "") ? "done" : "unmet", { target, value: clip(actual, 300) });
    }
    return result("error", { message: "this action has no requested control state" });
  };
  const typeInto = (el, text) => {
    if (el.focus) el.focus({ preventScroll: true });
    if (
      !fire(el, "beforeinput", "InputEvent", {
        bubbles: true,
        cancelable: true,
        inputType: "insertText",
        data: text,
      })
    ) {
      return false;
    }
    if (el.isContentEditable) {
      const doc = el.ownerDocument;
      const range = doc.createRange();
      range.selectNodeContents(el);
      const selection = viewOf(el).getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      let inserted;
      try {
        inserted = doc.execCommand("insertText", false, text);
      } catch {
        inserted = false;
      }
      if (!inserted) el.textContent = text;
    } else setValue(el, text);
    fire(el, "input", "InputEvent", {
      bubbles: true,
      inputType: "insertText",
      data: text,
    });
    fire(el, "change", "Event", { bubbles: true });
    return true;
  };
  // A key press as the browser does it for a real key: keydown, then (unless cancelled) the default action we can
  // reproduce: Enter submits the field's form or activates a button/link, Space toggles a checkbox, Tab moves focus.
  const press = (el, key) => {
    const init = {
      key,
      code: key.length === 1 ? `Key${key.toUpperCase()}` : key,
      bubbles: true,
      cancelable: true,
      composed: true,
    };
    const go = fire(el, "keydown", "KeyboardEvent", init);
    let effect = "none";
    if (go) {
      const tag = el.localName;
      const type = String(
        (el.getAttribute && el.getAttribute("type")) || "",
      ).toLowerCase();
      if (key === "Enter" && el.form && (tag === "input" || tag === "select")) {
        if (el.form.requestSubmit) el.form.requestSubmit();
        else el.form.submit();
        effect = "submitted the form";
      } else if (
        key === "Enter" &&
        (tag === "a" ||
          tag === "button" ||
          roleOf(el) === "button" ||
          roleOf(el) === "link")
      ) {
        el.click();
        effect = "activated";
      } else if (
        key === " " &&
        (type === "checkbox" || type === "radio" || roleOf(el) === "button")
      ) {
        el.click();
        effect = "toggled";
      } else if (key === "Tab") {
        const all = candidates().elements.filter(
          (c) =>
            !inFrame(c) &&
            visible(c) &&
            enabled(c) &&
            c.focus &&
            roleOf(c) !== "iframe",
        );
        const next = all[(all.indexOf(el) + 1) % Math.max(all.length, 1)];
        if (next) {
          next.focus();
          effect = "moved focus";
        }
      }
    } else effect = "the page cancelled the key";
    fire(el, "keyup", "KeyboardEvent", init);
    return effect;
  };
  try {
    const action = REQ.action;
    if (ENV) ENV.agentActing(3000);
    if (action === "scroll" && !REQ.ref) {
      const step =
        Number(REQ.amount) > 0
          ? Number(REQ.amount)
          : Math.round(window.innerHeight * 0.8);
      window.scrollBy(0, REQ.direction === "up" ? -step : step);
      return result("done", { scrollY: Math.round(window.scrollY) });
    }
    let el = null;
    if (REQ.ref) {
      const { found, target } = actionRef(REQ.ref);
      if (found.length === 0) return result("stale");
      if (!target) return result("ambiguous", { count: found.length });
      el = target;
    } else if (action === "press") {
      el = document.activeElement || document.body;
    } else return result("error", { message: "this action needs a ref" });
    const target = { role: roleOf(el), name: labelOf(el) };
    if (action === "type") {
      const secret = secretKind(el);
      if (secret) return result("secret", { kind: secret, target });
    }
    // A second host call reads the settled state. It sends no input, focus or scroll events.
    if (REQ.postconditionOnly) return postcondition(el, target);
    if (el.scrollIntoView && action !== "press" && !REQ.nativeValidateOnly)
      el.scrollIntoView({
        block: "center",
        inline: "center",
        behavior: "instant",
      });
    const { failed, covering, hitPoint, retryable } = checks(el, action);
    if (failed.length > 0)
      return result("notActionable", { failed, covering, target, retryable });
    if (REQ.nativeClick && ["click", "check"].includes(action)) {
      if (
        action === "check" &&
        (el.localName === "input"
          ? Boolean(el.checked)
          : el.getAttribute("aria-checked") === "true") ===
          (REQ.checked !== false)
      ) {
        return result("done", { target, checked: REQ.checked !== false });
      }
      const prepared = nativeClickPoint(el, hitPoint);
      if (prepared.failed)
        return result("notActionable", { ...prepared, target });
      return result("done", { ...prepared, target });
    }
    if (action === "click") {
      clickLike(el, hitPoint);
      return result("done", { target });
    }
    if (action === "hover") {
      const p = point(el, hitPoint);
      for (const [type, kind] of [["pointerover", "PointerEvent"], ["pointerenter", "PointerEvent"], ["mouseover", "MouseEvent"], ["mouseenter", "MouseEvent"], ["pointermove", "PointerEvent"], ["mousemove", "MouseEvent"]]) {
        fire(el, type, kind, p);
      }
      return result("done", { target });
    }
    if (action === "scroll")
      return result("done", { target, scrollY: Math.round(window.scrollY) });
    if (action === "type") {
      typeInto(el, String(REQ.text || ""));
      const answer = JSON.parse(postcondition(el, target));
      if (answer.status === "done" && REQ.submit) answer.effect = press(el, "Enter");
      return JSON.stringify(answer);
    }
    if (action === "select") {
      if (el.localName !== "select")
        return result("error", {
          message: "this element is not a select box (use click)",
          target,
        });
      const options = Array.from(el.options || []);
      const option = requestedOption(el);
      if (!option)
        return result("noOption", {
          options: options
            .slice(0, 30)
            .map((o) => clip(o.label || o.text || o.value, 80)),
          target,
        });
      if (el.focus) el.focus({ preventScroll: true });
      setValue(el, option.value);
      fire(el, "input", "Event", { bubbles: true });
      fire(el, "change", "Event", { bubbles: true });
      return postcondition(el, target);
    }
    if (action === "check") {
      const want = REQ.checked !== false;
      if (checkedState(el) !== want) clickLike(el, hitPoint);
      return postcondition(el, target);
    }
    if (action === "press") {
      // A checked text submission still requires the same value at the instant Enter is dispatched.
      if (REQ.expectedText !== undefined && String(el.isContentEditable ? el.innerText : el.value) !== REQ.expectedText)
        return result("unmet", { target });
      const key = String(REQ.key || "Enter");
      return result("done", { target, effect: press(el, key) });
    }
    return result("error", { message: `unknown action ${clip(action, 40)}` });
  } catch (error) {
    return result("error", {
      message: clip(error && error.message ? error.message : error, 300),
    });
  }
})();
