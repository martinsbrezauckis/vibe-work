/* Sandbox layout and comment-pick reporter. Depends on postMessage; served before prototype scripts by the page server.
 * Reports only geometry/picks and accepts read-only app snapshots. It has no answer-writing channel. */
(() => {
  const post = parent.postMessage.bind(parent);
  const send = (data) => post(data, "*");
  const boxes = new Map();
  let frame = 0;
  let pick = null;
  let previous = "";
  const measure = () => {
    frame = 0;
    const result = [];
    for (const [id, item] of boxes) {
      const node = document.getElementById(id);
      if (node !== item.node) {
        item.node = node;
        prepare(item);
      }
      const rect = node?.getBoundingClientRect();
      result.push({
        id,
        x: rect?.x ?? 0,
        y: rect?.y ?? 0,
        width: rect?.width ?? 0,
        height: rect?.height ?? 0,
      });
    }
    const json = JSON.stringify(result);
    if (json !== previous) {
      previous = json;
      send({ type: "hb-layout", boxes: result });
    }
  };
  const schedule = () => {
    if (!frame) frame = requestAnimationFrame(measure);
  };
  const prepare = (item) => {
    if (!item.node?.matches("div.hb-input[data-input]")) return;
    item.node.classList.add("hb-inline-slot");
    const height = `${Math.max(30, Math.min(10000, item.height))}px`;
    if (item.node.style.minHeight !== height)
      item.node.style.minHeight = height;
  };
  addEventListener("message", (event) => {
    if (event.source !== parent) return;
    const data = event.data;
    if (
      data?.type === "hb-layout-init" &&
      Array.isArray(data.inputs) &&
      data.inputs.length <= 500
    ) {
      if (data.theme === "light" || data.theme === "dark")
        document.documentElement.dataset.theme = data.theme;
      for (const input of data.inputs) {
        if (typeof input.id !== "string" || typeof input.height !== "number")
          continue;
        let item = boxes.get(input.id);
        if (!item) {
          item = { node: document.getElementById(input.id) };
          boxes.set(input.id, item);
        }
        item.height = input.height;
        prepare(item);
      }
      schedule();
    } else if (data?.type === "hb-pick") {
      pick = data.name;
      const selection = getSelection();
      if (selection && !selection.isCollapsed) {
        const node = selection.getRangeAt(0).commonAncestorContainer;
        picked(
          node.nodeType === 1 ? node : node.parentElement,
          String(selection),
        );
      }
    } else if (data?.type === "hb-pick-cancel") pick = null;
  });
  const picked = (node, quote) => {
    const ids = [];
    for (let n = node; n && ids.length < 8; n = n.parentElement) {
      if (n.id) ids.push(n.id.slice(0, 100));
      if (n.dataset.step && ids.length < 8)
        ids.push(n.dataset.step.slice(0, 100));
    }
    send({
      type: "hb-picked",
      name: pick,
      result: {
        state: "picked",
        ids,
        path: node.id ? `${node.localName}#${node.id}` : node.localName,
        tag: node.localName,
        role: node.getAttribute("role") || node.localName,
        text: (node.innerText || node.textContent || "").slice(0, 2000),
        quote: quote?.slice(0, 2000) ?? null,
        url: location.href,
      },
    });
    pick = null;
  };
  addEventListener(
    "click",
    (event) => {
      if (!pick) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      picked(event.target, null);
    },
    true,
  );
  addEventListener(
    "keydown",
    (event) => {
      if (!pick || event.key !== "Escape") return;
      send({ type: "hb-picked", name: pick, result: { state: "cancelled" } });
      pick = null;
    },
    true,
  );
  addEventListener(
    "DOMContentLoaded",
    () => {
      addEventListener("click", (event) => {
        const link = event.target.closest?.("a[href]");
        if (!event.isTrusted || event.defaultPrevented || !link) return;
        const url = new URL(link.href, location.href);
        if (
          url.pathname === location.pathname &&
          url.search === location.search &&
          url.hash
        )
          return;
        if (!["http:", "https:", "mailto:"].includes(url.protocol)) return;
        event.preventDefault();
        send({ type: "hb-navigate", href: url.href });
      });
      const style = document.createElement("style");
      style.textContent =
        ".hb-inline-slot{box-sizing:border-box!important;background:transparent!important;border-color:transparent!important}.hb-inline-slot>*{display:none!important}";
      document.head.append(style);
      new MutationObserver(schedule).observe(document.body, {
        subtree: true,
        childList: true,
        attributes: true,
        characterData: true,
      });
      new ResizeObserver(schedule).observe(document.body);
      send({ type: "hb-ready" });
      schedule();
    },
    { once: true },
  );
  addEventListener("scroll", schedule, true);
  addEventListener("resize", schedule);
  addEventListener("load", schedule, true);
})();
