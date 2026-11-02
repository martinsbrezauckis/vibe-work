// Scrolls one ancestor document so an element inside one of its child frames shows: a scroll {ref} inside another
// site's frame cannot move the documents around that frame (the engine keeps a cross-origin frame from scrolling
// its parents). Runs in the parent frame's isolated world; sends no input events.
// REQ: { page, node, box: {x, y, width, height} } (`node`: the child frame element's identity in this document;
// `box`: the element in the child frame's viewport). Answer: { status: done|stale, box, viewport } with the
// element's box in this document's viewport after the scroll. Synchronous.
/* global ENV, REQ -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  // eslint-disable-next-line no-undef -- Expanded by the script assembler.
  __VW_COMMON__;
  const frame = ENV?.nodeIdentities
    ? Array.from(document.querySelectorAll("iframe,frame")).find((el) => ENV.nodeIdentities.get(el) === REQ.node)
    : null;
  if (!frame) return JSON.stringify({ status: "stale" });
  const inView = () => {
    const r = frame.getBoundingClientRect();
    const sx = frame.offsetWidth ? r.width / frame.offsetWidth : 1;
    const sy = frame.offsetHeight ? r.height / frame.offsetHeight : 1;
    return {
      x: r.left + (frame.clientLeft + REQ.box.x) * sx,
      y: r.top + (frame.clientTop + REQ.box.y) * sy,
      width: REQ.box.width * sx,
      height: REQ.box.height * sy,
    };
  };
  const before = inView();
  const outside = (start, size, view) => start < 0 || start + size > view;
  // Centre the element along an axis where it does not fully show (as scrollIntoView's "center" does).
  const dx = outside(before.x, before.width, innerWidth) ? before.x + before.width / 2 - innerWidth / 2 : 0;
  const dy = outside(before.y, before.height, innerHeight) ? before.y + before.height / 2 - innerHeight / 2 : 0;
  if (dx || dy) window.scrollBy({ left: dx, top: dy, behavior: "instant" });
  const after = inView();
  const round = (b) => Object.fromEntries(Object.entries(b).map(([k, v]) => [k, Math.round(v)]));
  return JSON.stringify({ status: "done", box: round(after), viewport: [innerWidth, innerHeight] });
})();
