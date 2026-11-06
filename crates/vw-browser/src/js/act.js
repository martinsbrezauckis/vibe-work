// One agent action on a retained ref in this document: find the same node and label, check it is
// actionable, then act like a user would (pointer and key events, the native value setter so frameworks see the
// change). Synthetic events: the page environment never counts them as the user's typing.
// REQ: { page, action: click|type|select|check|press|hover|scroll, ref?, text?, value?, checked?, key?, submit?,
//        direction?, amount? }.
// Answer: { status: done|stale|ambiguous|secret|notActionable|noOption|error, ... }. Synchronous.
// Adapted from ShellX control script (native value setter, beforeinput/input/
// change, contenteditable insertText) and actionability checks; changed: Enter submits the form the way a
// browser does for a real key press, select uses the native setter.
/* global authorizationPage, ENV, MAX_FRAME_DEPTH, MAX_SHADOW_DEPTH, REQ, actionRef, candidates, clip, composedParent, consentSnapshot, describe, editable, enabled, globalRect, labelOf, labelledToggle, nativeClickPoint, observedTarget, refOf, roleOf, secretKind, viewOf, visibilityState, visible -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  /* global dragTo -- Inserted by the script assembler. */
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
  const authPage = authorizationPage();
  // The host approved one document and one ref. Every mutating route, including Enter and form submit, passes
  // through here before any event reaches the page. A changed document must be observed and classified again.
  if (
    (authPage || REQ.consent) &&
    !REQ.postconditionOnly &&
    !REQ.nativeUser &&
    !["hover", "scroll", "uploadChunk"].includes(REQ.action)
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
    drag: ["visible", "stable", "enabled", "receivesEvents"],
    check: ["visible", "stable", "enabled", "receivesEvents"],
    type: ["visible", "stable", "enabled", "editable"],
    select: ["visible", "stable", "enabled"],
    hover: ["visible", "stable", "receivesEvents"],
    press: ["visible", "enabled"],
    scroll: [],
    upload: ["enabled"],
  };
  // The file field a ref stands for: the field itself, the field of its label, or the single file field inside it,
  // inside its form, or on its page (a "Choose file" button that opens a hidden field).
  const fileField = (el) => {
    const isFile = (c) => c && c.localName === "input" && String(c.type).toLowerCase() === "file";
    if (isFile(el)) return el;
    if (el.localName === "label" && isFile(el.control)) return el.control;
    const label = el.closest && el.closest("label");
    if (label && isFile(label.control)) return label.control;
    for (const scope of [el, el.form || el.closest?.("form"), el.ownerDocument]) {
      const inside = scope ? Array.from(scope.querySelectorAll("input[type=file]")) : [];
      if (inside.length === 1) return inside[0];
      if (inside.length > 1) return null;
    }
    return null;
  };
  // Files staged by `uploadChunk` (base64 parts per file), set on the field by `upload`.
  const uploadFiles = (field) => {
    const staged = ENV.uploads?.get(REQ.token);
    if (!staged) return null;
    const View = viewOf(field);
    const transfer = new View.DataTransfer();
    for (const [i, file] of REQ.files.entries()) {
      const parts = (staged[i] || []).map((data) => Uint8Array.from(atob(data), (c) => c.charCodeAt(0)));
      if (parts.length !== file.parts) return null;
      transfer.items.add(new View.File(parts, file.name, { type: file.mime, lastModified: Date.now() }));
    }
    ENV.uploads.delete(REQ.token);
    return transfer.files;
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
  // eslint-disable-next-line no-undef -- Expanded by the Rust script assembler.
  __VW_DRAG__;
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
  const checkedState = (el) => {
    const control = labelledToggle(el) || el;
    return control.localName === "input" ? Boolean(control.checked) : control.getAttribute("aria-checked") === "true";
  };
  /* global requestedValue, rangeKey -- Inserted by the script assembler. */
  // eslint-disable-next-line no-undef -- Expanded by the Rust script assembler.
  __VW_WIDGET_DATES__;
  // eslint-disable-next-line no-undef -- Expanded by the Rust script assembler.
  __VW_FORM_VALUES__;
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
      const wanted = requestedValue(el, String(REQ.text || ""));
      return result(!wanted.status && actual === wanted.value ? "done" : "unmet", { target, value: clip(actual, 300) });
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
      code: /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : /^[0-9]$/.test(key) ? `Digit${key}` : key === " " ? "Space" : key,
      ...REQ.modifiers,
      bubbles: true,
      cancelable: true,
      composed: true,
    };
    const go = fire(el, "keydown", "KeyboardEvent", init);
    let effect = "none";
    const command = init.ctrlKey || init.metaKey || init.altKey;
    if (go && command && !init.altKey && !init.shiftKey && key.toLowerCase() === "a" && editable(el)) {
      if (typeof el.select === "function") el.select();
      else if (el.isContentEditable) {
        const range = el.ownerDocument.createRange();
        range.selectNodeContents(el);
        const selection = viewOf(el).getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
      }
      effect = "selected the field";
    }
    if (go && !command) {
      const tag = el.localName;
      const type = String(
        (el.getAttribute && el.getAttribute("type")) || "",
      ).toLowerCase();
      const rangeEffect = rangeKey(el, key);
      if (rangeEffect !== null) {
        effect = rangeEffect;
      } else if (key === "Enter" && !init.shiftKey && el.form && (tag === "input" || tag === "select")) {
        if (el.form.requestSubmit) el.form.requestSubmit();
        else el.form.submit();
        effect = "submitted the form";
      } else if (
        key === "Enter" && !init.shiftKey &&
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
            c.ownerDocument === el.ownerDocument &&
            visible(c) &&
            enabled(c) &&
            c.focus &&
            roleOf(c) !== "iframe",
        );
        const next = all[(all.indexOf(el) + (init.shiftKey ? -1 : 1) + all.length) % Math.max(all.length, 1)];
        if (next) {
          next.focus();
          effect = "moved focus";
        }
      }
    } else if (!go) effect = "the page cancelled the key";
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
    if (action === "uploadChunk") {
      // At most 48 MB of staged text per page document (two full uploads); a new token replaces older ones.
      ENV.uploads ||= new Map();
      if (!ENV.uploads.has(REQ.token)) ENV.uploads.clear();
      const staged = ENV.uploads.get(REQ.token) || [];
      const size = staged.flat().reduce((n, part) => n + part.length, 0) + String(REQ.data).length;
      if (size > 48 * 1024 * 1024) return result("error", { message: "the upload is too large" });
      (staged[REQ.file] ||= [])[REQ.part] = String(REQ.data);
      ENV.uploads.set(REQ.token, staged);
      return result("done");
    }
    let el = null;
    if (REQ.ref) {
      // A scroll also brings a canvas or image region of the read into view (it sends no input to it).
      const { found, target } = actionRef(REQ.ref, action === "scroll");
      if (found.length === 0) return result("stale");
      if (!target) return result("ambiguous", { count: found.length });
      // A ref hash describes markup, not a node. Retention must never retarget an identical replacement;
      // a changed label also needs a read so the host's prompt guard checks the words before input.
      if (!observedTarget(target, REQ.observedNode, REQ.observedName)) return result("stale");
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
    if (action === "type") {
      const wanted = requestedValue(el, String(REQ.text || ""));
      if (wanted.status) return result(wanted.status, { target, kind: wanted.kind });
    }
    // Native colour panels have no page refs and can block WKWebView evaluation. The value setter is
    // the supported route; reject before synthetic events or native click geometry can open the panel.
    const control = el.closest?.("label")?.control || el;
    if (!REQ.nativeUser && action === "click" && control?.localName === "input" && control.type === "color")
      return result("unsupportedPicker", { target });
    if (action === "press" && el.localName === "input" &&
        ["color", "date", "time", "datetime-local", "month", "week"].includes(el.type) &&
        !["Tab", "Escape"].includes(REQ.key) && (el.type === "color" || REQ.key !== "Enter"))
      return result("unsupportedNativeKey", { target });
    if (el.scrollIntoView && !["press", "upload"].includes(action) && !REQ.nativeValidateOnly)
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
        checkedState(el) === (REQ.checked !== false)
      ) {
        return result("done", { target, checked: REQ.checked !== false });
      }
      const prepared = nativeClickPoint(el, hitPoint);
      if (prepared.failed)
        return result("notActionable", { ...prepared, target });
      return result("done", { ...prepared, target });
    }
    if (action === "drag") return dragTo(el, target, hitPoint);
    if (action === "upload") {
      const field = fileField(el);
      if (!field) return result("notFileField", { target });
      const fieldTarget = { role: roleOf(field), name: labelOf(field) || target.name };
      if (!enabled(field)) return result("notActionable", { failed: ["enabled"], target: fieldTarget });
      if (REQ.files.length > 1 && !field.multiple) return result("notMultiple", { target: fieldTarget });
      const files = uploadFiles(field);
      if (!files) return result("error", { message: "the files did not all reach the page; try the upload again" });
      // As a person's file picker does: the field holds the files, then input and change.
      field.files = files;
      fire(field, "input", "Event", { bubbles: true, composed: true });
      fire(field, "change", "Event", { bubbles: true });
      const held = Array.from(field.files || []);
      return result(held.length === REQ.files.length ? "done" : "unmet", {
        target: fieldTarget,
        uploaded: held.map((f) => ({ name: clip(f.name, 200), size: f.size })),
      });
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
    if (action === "scroll") {
      // Where the element ended up in this document's viewport: the host moves the documents around another
      // site's frame, and never answers done for an element that stayed off screen.
      const r = el.getBoundingClientRect();
      return result("done", { target, scrollY: Math.round(window.scrollY),
        box: { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) },
        viewport: [innerWidth, innerHeight] });
    }
    if (action === "type") {
      const wanted = requestedValue(el, String(REQ.text || ""));
      if (wanted.status) return result(wanted.status, { target, kind: wanted.kind });
      if (typeInto(el, wanted.value) && wanted.widgetDate)
        fire(el, "keyup", "KeyboardEvent", { key: wanted.value.slice(-1), bubbles: true });
      const answer = JSON.parse(postcondition(el, target));
      if (answer.status === "done" && REQ.submit) answer.effect = press(el, "Enter");
      return JSON.stringify(answer);
    }
    if (action === "select") {
      if (["input", "textarea"].includes(el.localName))
        return result("unsupportedInputSelect", { target });
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
      return result("done", { target, effect: press(el, key), ref: refOf(el) });
    }
    return result("error", { message: `unknown action ${clip(action, 40)}` });
  } catch (error) {
    return result("error", {
      message: clip(error && error.message ? error.message : error, 300),
    });
  }
})();
