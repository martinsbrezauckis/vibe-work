// A text condition as a minimal frame model, validated and visibility-filtered by the native frame merger.
// Called only by text waits; returns a match marker, never page text or a new ref observation.
/* global REQ, hideKeys -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  // eslint-disable-next-line no-undef -- Rust replaces the following prelude marker before execution.
  __VW_COMMON__
  const body = document.body;
  const style = body && getComputedStyle(body);
  const rendered = style && style.display !== "none" && style.visibility === "visible" && Number(style.opacity) !== 0;
  const text = rendered ? hideKeys(String(body.innerText || "")) : "";
  const matched = text.toLowerCase().includes(String(REQ.text).toLowerCase());
  return JSON.stringify({ url: String(location.href).slice(0, 2000), readyState: document.readyState,
    text: matched ? "1" : "", textLength: Number(matched) });
})()
