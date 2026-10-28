/* Handbook fragment navigation for section ids and data-step values.
 * Installed by the viewer and prototype prelude; only scrolls the current document. */
(() => {
  const scroll = () => {
    let anchor;
    try {
      anchor = decodeURIComponent(location.hash.slice(1));
    } catch {
      return;
    }
    if (!anchor) return;
    const frame = document.querySelector("iframe[data-hb-prototype]");
    if (frame) {
      frame.contentWindow?.postMessage({ type: "hb-anchor", anchor }, "*");
      return;
    }
    const target =
      document.getElementById(anchor) ||
      document.querySelector(`[data-step="${CSS.escape(anchor)}"]`);
    target?.scrollIntoView({ block: "start" });
  };
  addEventListener("hashchange", scroll);
  addEventListener("DOMContentLoaded", scroll, { once: true });
  addEventListener("load", scroll);
  addEventListener("message", (event) => {
    if (
      event.source === parent &&
      window !== parent &&
      event.data?.type === "hb-anchor" &&
      typeof event.data.anchor === "string" &&
      /^[a-z0-9-]{1,100}$/.test(event.data.anchor)
    ) {
      location.hash = event.data.anchor;
      scroll();
    }
    const frame = document.querySelector("iframe[data-hb-prototype]");
    if (
      frame &&
      event.source === frame.contentWindow &&
      event.data?.type === "hb-ready"
    )
      scroll();
  });
})();
