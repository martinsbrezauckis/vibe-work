// Full-page screenshot helpers, one small synchronous step per call (the host waits between them):
// `metrics` (document and viewport size, scroll, device pixel ratio, pending images), `scroll` to REQ.y (answers
// where the page really is), `pin` (fixed and sticky elements: before every slice after the first they are made
// transparent, or they would repeat in every slice; ShellX Browser's stitch repeats them), `unpin`, and
// `eager` (images marked loading=lazy load now, so a one-render capture does not show empty boxes).
// REQ: { page, op, y }. The page's own scroll position is restored by the host at the end.
(() => {
  __VW_COMMON__
  const doc = document.documentElement;
  const body = document.body;
  const out = { url: String(location.href).slice(0, 2000) };
  const pending = () => Array.from(document.images || []).filter((img) => (img.currentSrc || img.src) && !img.complete).length;
  if (REQ.op === "metrics") {
    out.title = clip(document.title, 300);
    out.width = Math.ceil(Math.max(doc.scrollWidth, body ? body.scrollWidth : 0, doc.clientWidth));
    out.height = Math.ceil(Math.max(doc.scrollHeight, body ? body.scrollHeight : 0, doc.clientHeight));
    out.viewportWidth = window.innerWidth;
    out.viewportHeight = window.innerHeight;
    out.clientWidth = doc.clientWidth;
    out.scrollX = window.scrollX;
    out.scrollY = window.scrollY;
    out.dpr = window.devicePixelRatio || 1;
    out.pendingImages = pending();
  } else if (REQ.op === "scroll") {
    window.scrollTo({ left: 0, top: Number(REQ.y) || 0, behavior: "instant" });
    out.scrollY = window.scrollY;
    out.pendingImages = pending();
  } else if (REQ.op === "eager") {
    let changed = 0;
    for (const img of Array.from(document.querySelectorAll("img[loading=lazy]"))) {
      img.loading = "eager";
      changed += 1;
    }
    out.changed = changed;
    out.pendingImages = pending();
  } else if (REQ.op === "pin") {
    // Hidden with an attribute and an inline !important rule we own, restored by `unpin`.
    let pinned = 0;
    const { elements } = collect();
    for (const el of elements) {
      if (el.ownerDocument !== document || pinned >= 200) continue;
      const cs = getComputedStyle(el);
      if ((cs.position === "fixed" || cs.position === "sticky") && !el.hasAttribute("data-vw-capture-pin") && cs.visibility !== "hidden") {
        // The element's own style attribute as it was ("s:<text>", or "none"), restored exactly by `unpin`.
        el.setAttribute("data-vw-capture-pin", el.hasAttribute("style") ? `s:${el.getAttribute("style")}` : "none");
        // Opacity, not visibility: descendants can set `visibility: visible` and stay shown (a OneTrust cookie
        // banner did); nothing inside can undo an ancestor's opacity.
        el.style.setProperty("opacity", "0", "important");
        pinned += 1;
      }
    }
    out.pinned = pinned;
  } else if (REQ.op === "unpin") {
    let restored = 0;
    for (const el of Array.from(document.querySelectorAll("[data-vw-capture-pin]"))) {
      const original = String(el.getAttribute("data-vw-capture-pin"));
      el.removeAttribute("data-vw-capture-pin");
      // Chromium writes the inline style back into the attribute lazily: read it once so the pending write happens
      // now, before the attribute is replaced (otherwise a later read brings back style="").
      void el.getAttribute("style");
      if (original.startsWith("s:")) el.setAttribute("style", original.slice(2));
      else el.removeAttribute("style");
      restored += 1;
    }
    out.restored = restored;
  }
  return JSON.stringify(out);
})()
