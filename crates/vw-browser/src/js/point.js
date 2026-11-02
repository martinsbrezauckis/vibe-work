// Final point guard, evaluated in native frames and consumed only by the native input callback.
// Observed node, frame document, viewport, geometry and hit target must all still agree.
/* global REQ, authorizationPage, consentSnapshot, armPointReceipt -- Supplied by the Rust assembler. */
(() => {
  // eslint-disable-next-line no-undef -- Expanded by the script assembler.
  __VW_COMMON__
  // eslint-disable-next-line no-undef -- Expanded by the script assembler.
  __VW_POINT_RECEIPT__
  const request = REQ;
  const env = window[request.page];
  const check = request.checks.find(c => c.document === env?.frameDocument);
  if (!check) { armPointReceipt(null, null); return null; }
  const fail = reason => JSON.stringify({ document:check.document, ok:false, reason });
  if (innerWidth !== check.viewport[0] || innerHeight !== check.viewport[1]) return fail("viewport changed");
  if (visualViewport && (visualViewport.scale !== 1 || visualViewport.offsetLeft || visualViewport.offsetTop)) return fail("visual viewport changed");
  if (check.x < 0 || check.y < 0 || check.x >= innerWidth || check.y >= innerHeight) return fail("point outside frame");
  let hit = document.elementFromPoint(check.x, check.y);
  for (let i = 0; hit?.shadowRoot && i < 8; i += 1) {
    const next = hit.shadowRoot.elementFromPoint(check.x, check.y);
    if (!next || next === hit) break;
    hit = next;
  }
  let target = hit;
  for (let i = 0; target && env.nodeIdentities?.get(target) !== check.node && i < 64; i += 1)
    target = target.parentElement || target.getRootNode()?.host;
  if (!target || !check.node || env.nodeIdentities?.get(target) !== check.node) return fail("node or frame under point changed");
  if (check.target && !REQ.nativeUser && (authorizationPage() || REQ.consent)) {
    if (!REQ.consent || REQ.consent.snapshot !== consentSnapshot(REQ.ref).snapshot) return fail("authorization changed");
  }
  const rect = target.getBoundingClientRect();
  if (["x", "y", "width", "height"].some(k => Math.round(rect[k]) !== check.bounds[k])) return fail("target geometry changed");
  for (let node = target; node; node = node.parentElement || node.getRootNode()?.host) {
    const style = getComputedStyle(node);
    if (node.inert || node.disabled || node.getAttribute("aria-disabled") === "true" || style.visibility !== "visible" || Number(style.opacity) === 0) return fail("target is not actionable");
    if (style.perspective !== "none" || (style.rotate && !["none", "0deg"].includes(style.rotate))) return fail("transformed target");
    if (style.transform !== "none") {
      const matrix = new DOMMatrixReadOnly(style.transform);
      if (!matrix.is2D || matrix.a <= 0 || matrix.d <= 0 || Math.abs(matrix.b) > 0.00001 || Math.abs(matrix.c) > 0.00001) return fail("transformed target");
    }
  }
  armPointReceipt(check, target);
  return JSON.stringify({ document:check.document, ok:true, armed:REQ.receipt });
})()
