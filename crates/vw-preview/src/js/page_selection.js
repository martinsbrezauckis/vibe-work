// The current document's selection, retained when native Browser chrome takes focus.
// Included inside page_init.js; only the isolated trusted bridge keeps text and DOM ranges.
/* global TRUSTED */
/* exported pageSelection */
const pageSelection = (() => {
  if (!TRUSTED) return () => null;
  // The native script builder enables this only for WKWebView. On X11, restoring an
  // unfocused range takes PRIMARY ownership and can clear a selection in Browser chrome.
  const RESTORE_SELECTION = false;
  let saved = null;
  let leaving = false;
  const clear = () => { saved = null; leaving = false; };
  const invalidate = () => { saved = null; leaving = true; };
  const capture = () => {
    if (saved && saved.url !== String(location.href)) { invalidate(); return; }
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed && selection.rangeCount) {
      const range = selection.getRangeAt(0).cloneRange();
      const quote = String(range);
      saved = quote.trim() && quote.length <= 100000
        ? { range, quote, node: range.commonAncestorContainer, url: String(location.href) } : null;
    }
  };
  const valid = () => saved && saved.url === String(location.href) && saved.node.isConnected &&
    saved.node.ownerDocument === document && String(saved.range) === saved.quote;
  // WKWebView removes the live range when Browser chrome takes first responder, so the person's highlight vanishes
  // although the retained choice still works. Put the same range back while the page is unfocused: WebKit then
  // draws it as an inactive selection. Engines that keep their range on blur never reach the collapsed branch.
  const show = () => {
    if (!RESTORE_SELECTION || !leaving || document.hasFocus() || !valid()) return;
    const selection = window.getSelection();
    if (!selection || (selection.rangeCount && !selection.isCollapsed)) return;
    selection.removeAllRanges();
    selection.addRange(saved.range.cloneRange());
  };
  document.addEventListener("selectionchange", () => {
    if (!leaving) capture();
    else show();
  });
  const leave = () => {
    if (!leaving) capture();
    leaving = true;
    setTimeout(show, 0);
  };
  window.addEventListener("blur", leave);
  document.addEventListener("focusout", () => { if (!document.hasFocus()) leave(); });
  // Returning focus alone must not replace the retained range with WKWebView's collapsed selection.
  // Empty selectionchange may precede blur on WebKit. Only a fresh page gesture clears the old choice.
  window.addEventListener("pointerdown", (event) => { if (event.button === 0) clear(); }, true);
  window.addEventListener("keydown", (event) => {
    const command = event.metaKey || event.ctrlKey;
    if ((!command && !event.altKey && event.key.length === 1) ||
        (command && event.key.toLowerCase() === "a") ||
        ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown", "Backspace", "Delete"].includes(event.key)) clear();
  }, true);
  window.addEventListener("pagehide", invalidate);
  window.addEventListener("pageshow", (event) => { if (event.persisted) invalidate(); });
  window.addEventListener("popstate", invalidate);
  window.addEventListener("hashchange", invalidate);
  return (contextEvent) => {
    if (!leaving) capture();
    if (saved && !valid()) invalidate();
    if (saved && contextEvent?.button === 2) {
      const containsPoint = () => saved && Array.from(saved.range.getClientRects()).some((rect) =>
        contextEvent.clientX >= rect.left && contextEvent.clientX <= rect.right &&
        contextEvent.clientY >= rect.top && contextEvent.clientY <= rect.bottom);
      // A right click outside the old highlight can select a different word in the same paragraph.
      if (!containsPoint()) {
        clear();
        capture();
        if (!containsPoint()) clear();
      }
    }
    return saved;
  };
})();
