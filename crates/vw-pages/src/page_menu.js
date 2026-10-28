/* The page's right click opens the app's own menu (Point Vibe at this, Comment…, Copy). Depends on the native menu
 * poll and the menu pick/copy scripts (page_menu.rs, pick.rs). Loaded at document start by PagesService in the
 * viewer's top document only; page scripts never share this realm. Only a trusted right click is reported, once, as
 * a point and whether text was selected: no page text leaves until the person chooses an item. */
(() => {
  if (window.top !== window || window.origin === "null") return;
  let pending = null;
  let target = null;
  window.addEventListener(
    "contextmenu",
    (event) => {
      if (!event.isTrusted) return;
      const element = event.target instanceof Element ? event.target : event.target?.parentElement;
      // A text field keeps the system's own menu (paste, spelling).
      if (element?.closest?.("input,textarea,select,[contenteditable]")) return;
      event.preventDefault();
      const selection = window.getSelection();
      const selected =
        !!selection &&
        !selection.isCollapsed &&
        String(selection).trim() !== "" &&
        (element ? selection.containsNode(element, true) : false);
      // The selection's text as shown (line breaks between blocks), kept in the page until an item asks.
      target = {
        range: selected ? selection.getRangeAt(0).cloneRange() : null,
        text: selected ? String(selection) : "",
        element: element || document.body,
      };
      pending = {
        x: Math.round(event.clientX),
        y: Math.round(event.clientY),
        selected,
        at: Date.now(),
      };
    },
    true,
  );
  Object.defineProperty(window, "__vw_page_menu", {
    value: Object.freeze({
      // One-use: the newest right click within five seconds, as `{"x","y","selected"}`, else "".
      take: () => {
        const p = pending;
        pending = null;
        if (!p || Date.now() - p.at > 5000) return "";
        return JSON.stringify({ x: p.x, y: p.y, selected: p.selected });
      },
      // What the last right click was on, for the menu's items (kept until the next right click).
      target: () => target,
    }),
    enumerable: false,
    configurable: false,
    writable: false,
  });
})();
