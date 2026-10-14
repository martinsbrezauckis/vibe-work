// "Annotate this page": the USER points at an element (hover outline, click) or draws a box (drag) in
// the Browser tab; the answer says what it is (ref, role, name, text, DOM path) and where (viewport CSS pixels) so
// the host can crop a screenshot. Escape cancels. The overlay never sends anything anywhere; the host polls.
// REQ: { page, name (accessor of this annotation), op: start|poll|cancel, mode: element|box }.
// Adapted from Vibe Work's own pick script (vw-pages `pick.rs`): capture-phase listeners, default prevented,
// the overlay removed when it answers.
(() => {
  __VW_COMMON__
  const NAME = String(REQ.name);
  const reply = (v) => JSON.stringify(v);
  if (REQ.op === "poll") {
    const f = window[NAME];
    return typeof f === "function" ? f() : reply({ state: "gone" });
  }
  if (REQ.op === "cancel") {
    const stop = window[NAME + "_stop"];
    if (typeof stop === "function") stop();
    return reply({ state: "cancelled" });
  }
  const old = window[NAME + "_stop"];
  if (typeof old === "function") old();
  const st = { result: { state: "pending" } };
  const view = { width: Math.round(window.innerWidth), height: Math.round(window.innerHeight) };
  const box = document.createElement("div");
  box.style.cssText = "position:fixed;pointer-events:none;z-index:2147483647;outline:2px solid #fa500f;" +
    "background:#fa500f1a;border-radius:3px;display:none";
  document.documentElement.appendChild(box);
  const show = (x, y, w, h) => {
    box.style.display = "block";
    box.style.left = x + "px";
    box.style.top = y + "px";
    box.style.width = w + "px";
    box.style.height = h + "px";
  };
  // A hint to the source file, only when the page's build put one there (no guessing): React/Vite inspector plugins
  // and similar dev tools write data attributes like these on elements or their ancestors.
  const SOURCE_ATTRS = ["data-inspector-relative-path", "data-insp-path", "data-v-inspector", "data-source-file",
    "data-sourcefile", "data-source", "data-loc", "data-component-file", "data-file"];
  const sourceOf = (el) => {
    for (let cur = el, depth = 0; cur && cur.nodeType === 1 && depth < 12; cur = cur.parentElement, depth += 1) {
      for (const name of SOURCE_ATTRS) {
        const value = cur.getAttribute(name);
        if (value) {
          const line = cur.getAttribute("data-inspector-line");
          return clip(line ? `${value}:${line}` : value, 200);
        }
      }
    }
    return null;
  };
  const describeAt = (el, rect) => {
    const target = el && el.closest ? el.closest(INTERACTIVE) || el : el;
    return {
      state: "picked",
      mode: REQ.mode,
      ref: target && target.nodeType === 1 && matches(target, INTERACTIVE) ? refOf(target) : null,
      role: target ? roleOf(target) : "",
      name: target ? labelOf(target) : "",
      text: target ? clip(target.innerText || target.textContent, 500) : "",
      path: target ? domPath(target) : "",
      url: String(location.href).slice(0, 500),
      route: clip(location.pathname + location.search + location.hash, 300),
      source: target ? sourceOf(target) : null,
      rect: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) },
      viewport: view,
    };
  };
  let start = null;
  const move = (e) => {
    if (REQ.mode === "box") {
      if (!start) return;
      const x = Math.min(start.x, e.clientX), y = Math.min(start.y, e.clientY);
      show(x, y, Math.abs(e.clientX - start.x), Math.abs(e.clientY - start.y));
      return;
    }
    const r = e.target && e.target.getBoundingClientRect && e.target.getBoundingClientRect();
    if (r) show(r.left, r.top, r.width, r.height);
  };
  const down = (e) => {
    if (REQ.mode !== "box") return;
    e.preventDefault();
    e.stopImmediatePropagation();
    start = { x: e.clientX, y: e.clientY };
  };
  const up = (e) => {
    if (REQ.mode !== "box" || !start) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const rect = { x: Math.min(start.x, e.clientX), y: Math.min(start.y, e.clientY), width: Math.abs(e.clientX - start.x), height: Math.abs(e.clientY - start.y) };
    start = null;
    if (rect.width < 8 || rect.height < 8) return;
    box.style.display = "none";
    const centre = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
    st.result = describeAt(centre, rect);
    stop();
  };
  const click = (e) => {
    e.preventDefault();
    e.stopImmediatePropagation();
    if (REQ.mode === "box") return;
    const r = e.target.getBoundingClientRect();
    st.result = describeAt(e.target, { x: r.left, y: r.top, width: r.width, height: r.height });
    stop();
  };
  const key = (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      st.result = { state: "cancelled" };
      stop();
    }
  };
  const stop = () => {
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("mousedown", down, true);
    window.removeEventListener("mouseup", up, true);
    window.removeEventListener("click", click, true);
    window.removeEventListener("keydown", key, true);
    box.remove();
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("mousedown", down, true);
  window.addEventListener("mouseup", up, true);
  window.addEventListener("click", click, true);
  window.addEventListener("keydown", key, true);
  Object.defineProperty(window, NAME + "_stop", {
    value: () => {
      if (st.result.state === "pending") st.result = { state: "cancelled" };
      stop();
    },
    configurable: true,
  });
  Object.defineProperty(window, NAME, { value: () => reply(st.result), configurable: true });
  return reply(st.result);
})()
