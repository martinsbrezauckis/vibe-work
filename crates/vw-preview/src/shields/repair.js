// Cosmetic repair in a private Chromium world. Called by the native shields controller after its stylesheet.
// Page scripts cannot disconnect the observer; Off restores only inline display values that we changed.
(() => {
  globalThis.__vwShieldsStop?.();
  // eslint-disable-next-line no-undef -- replaced with serialized JSON by the native host before evaluation.
  const selectors = __SELECTORS__;
  const saved = new Map();
  const valid = selectors.filter((selector) => {
    try { return CSS.supports(`selector(${selector})`); } catch { return false; }
  });
  if (!valid.length) return;
  const selector = valid.join(",");
  const restore = (element, value) => {
    if (element.style.getPropertyValue("display") === "none" && element.style.getPropertyPriority("display") === "important") {
      if (value[0]) element.style.setProperty("display", value[0], value[1]);
      else element.style.removeProperty("display");
    }
  };
  const repair = (element) => {
    if (!(element instanceof Element) || !element.style) return;
    if (!element.matches(selector)) {
      if (saved.has(element)) { restore(element, saved.get(element)); saved.delete(element); }
      return;
    }
    // Native CSS hides ordinary nodes. Only a competing inline declaration needs repair.
    if (element.style.getPropertyValue("display") &&
        (element.style.getPropertyValue("display") !== "none" || element.style.getPropertyPriority("display") !== "important")) {
      saved.set(element, [element.style.getPropertyValue("display"), element.style.getPropertyPriority("display")]);
      element.style.setProperty("display", "none", "important");
    }
  };
  const scan = (node) => {
    repair(node);
    node.querySelectorAll?.("[style]").forEach(repair);
  };
  const observer = new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === "attributes") repair(record.target);
      else record.addedNodes.forEach(scan);
    }
    for (const element of saved.keys()) if (!element.isConnected) saved.delete(element);
  });
  observer.observe(document, { subtree: true, childList: true, attributes: true });
  scan(document);
  globalThis.__vwShieldsStop = () => {
    observer.disconnect();
    for (const [element, value] of saved) restore(element, value);
    saved.clear();
    delete globalThis.__vwShieldsStop;
  };
})();
