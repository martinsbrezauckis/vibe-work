// Prepare a native pointer point, without dispatching DOM events. Depends on common helpers;
// act.js calls this only for host-authorized clicks/checks. Native engines validate their own target window.
/* global ENV, MAX_FRAME_DEPTH, MAX_SHADOW_DEPTH, enabled, frameDoc, viewOf, visible -- Supplied by the Rust script assembler and its shared prelude. */
/* exported nativeClickPoint */
const nativeClickPoint = (el, admittedPoint = null) => {
  const fail = (reason) => ({ failed: [reason] });
  const axisAligned = (node) => {
    for (let cur = node; cur; cur = cur.parentElement) {
      const style = viewOf(cur).getComputedStyle(cur);
      if (style.perspective && style.perspective !== "none") return false;
      if (style.rotate && style.rotate !== "none" && style.rotate !== "0deg")
        return false;
      if (
        style.scale &&
        style.scale !== "none" &&
        style.scale
          .split(/\s+/)
          .some(
            (value) => !Number.isFinite(Number(value)) || Number(value) <= 0,
          )
      )
        return false;
      if (style.transform && style.transform !== "none") {
        const m = new DOMMatrixReadOnly(style.transform);
        if (
          !m.is2D ||
          m.a <= 0 ||
          m.d <= 0 ||
          Math.abs(m.b) > 0.00001 ||
          Math.abs(m.c) > 0.00001
        )
          return false;
      }
    }
    return true;
  };
  const hitAt = (doc, x, y, target) => {
    let hit = doc.elementFromPoint(x, y);
    for (
      let depth = 0;
      hit?.shadowRoot && depth < MAX_SHADOW_DEPTH;
      depth += 1
    ) {
      const next = hit.shadowRoot.elementFromPoint(x, y);
      if (!next || next === hit) break;
      hit = next;
    }
    for (let depth = 0; hit && depth < 16; depth += 1) {
      if (hit === target || target.contains(hit)) return true;
      hit = hit.getRootNode()?.host;
    }
    return false;
  };
  if (!axisAligned(el))
    return fail("native click requires an axis-aligned target");
  if (["iframe", "frame"].includes(el.localName) && !frameDoc(el))
    return fail("native click cannot inspect this cross-origin frame");
  const box = el.getBoundingClientRect();
  let x = admittedPoint?.x ?? box.left + box.width / 2,
    y = admittedPoint?.y ?? box.top + box.height / 2;
  let doc = el.ownerDocument,
    target = el;
  for (let depth = 0; depth <= MAX_FRAME_DEPTH; depth += 1) {
    const v = doc.defaultView;
    if (
      !v ||
      (v.visualViewport &&
        (v.visualViewport.scale !== 1 ||
          v.visualViewport.offsetLeft !== 0 ||
          v.visualViewport.offsetTop !== 0))
    ) {
      return fail("native click viewport changed; read the page again");
    }
    if (
      x < 0 ||
      y < 0 ||
      x >= v.innerWidth ||
      y >= v.innerHeight ||
      !hitAt(doc, x, y, target)
    )
      return fail("receivesEvents");
    if (doc === document) break;
    if (depth === MAX_FRAME_DEPTH)
      return fail("native click frame depth exceeded");
    const frame = v.frameElement;
    if (!frame || !visible(frame) || !enabled(frame) || !axisAligned(frame))
      return fail("native click frame changed or transformed");
    const r = frame.getBoundingClientRect();
    if (
      x >= frame.clientWidth ||
      y >= frame.clientHeight ||
      !frame.offsetWidth ||
      !frame.offsetHeight
    )
      return fail("receivesEvents");
    x = r.left + ((frame.clientLeft + x) * r.width) / frame.offsetWidth;
    y = r.top + ((frame.clientTop + y) * r.height) / frame.offsetHeight;
    target = frame;
    doc = frame.ownerDocument;
  }
  if (doc !== document || !Number.isFinite(x) || !Number.isFinite(y))
    return fail("native click frame changed");
  // Frame movement is checked as well as the target's local box. Wait for 120 ms of stable top-level geometry.
  if (!ENV.nativeClickStability) ENV.nativeClickStability = new WeakMap();
  const prior = ENV.nativeClickStability.get(el),
    now = performance.now();
  const geometry = [x, y, window.innerWidth, window.innerHeight];
  const same =
    prior &&
    prior.geometry.every((value, i) => Math.abs(value - geometry[i]) < 0.01);
  if (!same) ENV.nativeClickStability.set(el, { geometry, at: now });
  if (!same || now - prior.at < 120) return fail("stable");
  return {
    nativePoint: {
      x,
      y,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
    },
  };
};
