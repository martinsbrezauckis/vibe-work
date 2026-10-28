// After the agent worked on the user's annotations: is each annotated element still there and does it
// show the same text? REQ: { page, items: [{ ref?, text }] }. Answer: { states: ["changed"|"unchanged"|"missing"] }.
// An item without a ref (a drawn box) is "unchanged" while its text is still on the page, else "changed".
/* global REQ, clip, findRef -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  __VW_COMMON__
  let body = "";
  try {
    body = String((document.body && document.body.innerText) || "");
  } catch {
    body = "";
  }
  const states = (Array.isArray(REQ.items) ? REQ.items : []).slice(0, 50).map((item) => {
    const before = clip(item && item.text, 500);
    if (item && item.ref) {
      const found = findRef(String(item.ref));
      if (found.length !== 1) return "missing";
      return clip(found[0].innerText || found[0].textContent, 500) === before ? "unchanged" : "changed";
    }
    return before && body.replace(/\s+/g, " ").includes(before) ? "unchanged" : "changed";
  });
  return JSON.stringify({ states });
})()
