// Event-time target evidence for point.js, held only in the agent's isolated frame world.
// Only one trusted sequence after the host's dispatch mark can confirm input; overlap stays ambiguous.
/* global REQ, describe, hideKeys -- Supplied by the point script assembler. */
/* exported armPointReceipt */
const armPointReceipt = (check, target) => {
  const key = REQ.page + "_pointReceipt";
  window[key]?.clear();
  const types = ["mousedown", "mouseup", "click"];
  const result = { token: REQ.receipt, document: window[REQ.page]?.frameDocument,
    dispatchedAt: null, ambiguous: false, events: [] };
  let timer;
  const clear = () => {
    for (const type of types) window.removeEventListener(type, record, true);
    clearTimeout(timer);
    if (window[key]?.token === REQ.receipt) delete window[key];
  };
  const record = event => {
    if (!event.isTrusted) return;
    // Event creation time also catches input queued before the mark but delivered after it.
    if (result.dispatchedAt === null || !Number.isFinite(event.timeStamp)
      || event.timeStamp <= result.dispatchedAt || event.button !== 0
      || event.type !== types[result.events.length]) {
      result.ambiguous = true;
      return;
    }
    const path = event.composedPath();
    const actual = path.find(node => node?.nodeType === 1);
    const description = describe(actual);
    result.events.push({ type: event.type, timeStamp: event.timeStamp, matches: Boolean(check?.target && target && path.includes(target)
      && Math.abs(event.clientX - check.x) <= 2 && Math.abs(event.clientY - check.y) <= 2),
    role: hideKeys(description?.role || "element", actual).slice(0, 30),
    name: hideKeys(description?.name || "", actual).slice(0, 160) });
  };
  Object.defineProperty(window, key, { value: { token: REQ.receipt, result, clear }, configurable: true });
  for (const type of types) window.addEventListener(type, record, true);
  timer = setTimeout(clear, 6000);
};
