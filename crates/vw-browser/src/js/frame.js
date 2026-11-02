// Per-frame browser environment and document guard. Assembled by Rust, evaluated only through native frame APIs.
// Frame paths use WindowProxy identity, which is available across origins; page messages are never trusted.
// A child frame that cannot be observed answers `{unreadable:true}` (the host leaves it out); the main frame throws.
// A frame already removed from its page answers `{detached:true}` and is not counted at all.
/* global FRAME_REQUEST, FRAME_SOURCE -- Supplied by the Rust assembler. */
(() => {
  const req = FRAME_REQUEST;
  const run = () => {
  let env = window[req.page];
  if (!env && req.observe) {
    env = { stability: new WeakMap(), agentActing: () => {}, mutations: 0 };
    Object.defineProperty(window, req.page, { value: env });
    new MutationObserver(() => { env.mutations += 1; }).observe(document, { subtree: true, childList: true, attributes: true });
  }
  if (!env) throw new Error("frame document is stale or detached");
  if (!env.frameDocument) Object.defineProperty(env, "frameDocument", { value:
    Array.from(crypto.getRandomValues(new Uint32Array(4)), n => n.toString(16)).join("-") });
  // The observation keeps at most 2,000 characters of an address (a long data: address).
  if (req.document && (env.frameDocument !== req.document || String(location.href).slice(0, 2000) !== req.url))
    throw new Error("frame document changed since browser_read");
  const value = JSON.parse(FRAME_SOURCE);
  if (!req.observe) return JSON.stringify(value);
  const path = [];
  let current = window;
  for (let depth = 0; current !== current.top && depth < 8; depth += 1) {
    const parent = current.parent;
    let index = -1;
    for (let i = 0; i < parent.length && i < 64; i += 1) if (parent[i] === current) { index = i; break; }
    if (index < 0) throw new Error("frame ancestry is unavailable");
    path.unshift(index);
    current = parent;
  }
  if (current !== current.top) throw new Error("frame depth exceeded");
  const children = [];
  const box = r => ({ x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) });
  for (const el of document.querySelectorAll("iframe,frame")) {
    let index = -1;
    for (let i = 0; i < window.length && i < 64; i += 1) if (window[i] === el.contentWindow) { index = i; break; }
    const rect = el.getBoundingClientRect();
    const sx = el.offsetWidth ? rect.width / el.offsetWidth : 1, sy = el.offsetHeight ? rect.height / el.offsetHeight : 1;
    let visible = rect.width > 0 && rect.height > 0;
    for (let up = el; up; up = up.parentElement || up.getRootNode()?.host) {
      const style = getComputedStyle(up);
      if (style.visibility !== "visible" || style.display === "none" || Number(style.opacity) === 0) visible = false;
    }
    children.push({ index, visible, node: env.nodeIdentities?.get(el) || 0, bounds:box(rect),
      content:box({ x:rect.x + el.clientLeft * sx, y:rect.y + el.clientTop * sy,
        width:el.clientWidth * sx, height:el.clientHeight * sy }),
      x: rect.x + el.clientLeft * sx, y: rect.y + el.clientTop * sy,
      sx, sy, width: el.clientWidth, height: el.clientHeight });
  }
  // Every child window counts, also one the listing above maps to no element. A frame inside a shadow root is no
  // child window at all (`window.length` counts the document tree only), so it can never be read: the frames in
  // open shadow roots are counted too. A closed shadow root cannot be entered from any script world.
  let shadowed = 0;
  const roots = [document];
  for (let r = 0; r < roots.length; r += 1) for (const el of roots[r].querySelectorAll("*")) if (el.shadowRoot) {
    roots.push(el.shadowRoot);
    shadowed += el.shadowRoot.querySelectorAll("iframe,frame").length;
  }
  return JSON.stringify({ model: value, origin: String(window.origin), path, children, document: env.frameDocument,
    windows: Math.min(window.length, 64), shadowed: Math.min(shadowed, 64), width: innerWidth, height: innerHeight });
  };
  if (!req.observe || window === window.top) return run();
  // A removed frame's window can still answer from its detached document (WebKit on macOS keeps the handle a while).
  // It has no parent and is no part of the page any more: the host leaves it out without naming it unreadable. Every
  // attached child window has a parent in every script world, so no frame of the page can answer this.
  if (window.parent === null) return JSON.stringify({ detached: true });
  try { return run(); } catch { return JSON.stringify({ unreadable: true }); }
})()
