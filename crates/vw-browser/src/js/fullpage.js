// Full-page screenshot helpers, one small synchronous step per call (the host waits between them):
// `metrics` (document and viewport size, scroll, device pixel ratio, pending images), `scroll` to REQ.y (answers
// where the page really is), `pin` (fixed and sticky elements: before every slice after the first they are made
// transparent, or they would repeat in every slice; ShellX Browser's stitch repeats them), `unpin`, and
// `eager` (images marked loading=lazy load now, so a one-render capture does not show empty boxes).
// REQ: { page, op, y, capture? }. A capture token owns the scroll and styles until the host restores them.
(() => {
  __VW_COMMON__
  const doc = document.documentElement;
  const body = document.body;
  const out = { url: String(location.href).slice(0, 2000) };
  if (REQ.capture !== undefined) {
    if (!ENV) return JSON.stringify({ ...out, error: "the capture document closed" });
    if (REQ.op === "begin") {
      if (ENV.fullpageCapture) return JSON.stringify({ ...out, error: "another capture is still restoring this page" });
      ENV.fullpageCapture = { token: REQ.capture, x: scrollX, y: scrollY, pins: new Map() };
    } else if (ENV.fullpageCapture?.token !== REQ.capture) {
      return JSON.stringify({ ...out, error: "the capture document changed or was already restored" });
    }
  }
  const owned = REQ.capture !== undefined ? ENV.fullpageCapture : null;
  const unpin = () => {
    // Owned elements also cover open shadow roots and nodes detached while the capture was running.
    const elements = owned ? Array.from(owned.pins.keys()) : Array.from(document.querySelectorAll("[data-vw-capture-pin]"));
    for (const el of elements) {
      const original = owned ? owned.pins.get(el) : el.getAttribute("data-vw-capture-pin");
      el.removeAttribute("data-vw-capture-pin");
      // Flush Chromium's pending inline-style serialization before restoring the original attribute.
      void el.getAttribute("style");
      if (original !== null && original.startsWith("s:")) el.setAttribute("style", original.slice(2));
      else el.removeAttribute("style");
    }
    if (owned) owned.pins.clear();
    return elements.length;
  };
  const pending = () => Array.from(document.images || []).filter((img) => (img.currentSrc || img.src) && !img.complete).length;
  if (REQ.op === "metrics" || REQ.op === "begin") {
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
        const original = el.hasAttribute("style") ? `s:${el.getAttribute("style")}` : "none";
        if (owned) owned.pins.set(el, original);
        el.setAttribute("data-vw-capture-pin", original);
        // Opacity, not visibility: descendants can set `visibility: visible` and stay shown (a OneTrust cookie
        // banner did); nothing inside can undo an ancestor's opacity.
        el.style.setProperty("opacity", "0", "important");
        pinned += 1;
      }
    }
    out.pinned = pinned;
  } else if (REQ.op === "unpin") {
    out.restored = unpin();
  } else if (REQ.op === "restore" && owned) {
    out.restored = unpin();
    window.scrollTo({ left: owned.x, top: owned.y, behavior: "instant" });
    delete ENV.fullpageCapture;
  }
  return JSON.stringify(out);
})()
