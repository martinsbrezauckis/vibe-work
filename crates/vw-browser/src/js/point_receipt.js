// Event-time target evidence for point.js, held only in the agent's isolated frame world.
// Only one trusted sequence after the host's dispatch mark can confirm input; overlap stays ambiguous.
/* global REQ, describe, hideKeys -- Supplied by the point script assembler. */
/* exported armPointReceipt */
const armPointReceipt = (check, target) => {
  const key = REQ.page + "_pointReceipt";
  window[key]?.clear();
  const expected = REQ.expected || ["mousedown", "mouseup", "click"];
  const types = [...new Set(expected)];
  const options = REQ.click || { button: 0, count: 1, modifiers: 0 };
  const result = { token: REQ.receipt, document: window[REQ.page]?.frameDocument,
    dispatchedAt: null, ambiguous: false, events: [] };
  let timer;
  let contextPress = null;
  // The host menu runs before page handlers. Let this native sequence reach them, but never open a menu
  // that can steal focus from the next act. Match its press, not just a broad "agent is busy" interval.
  const takeContextMenu = event => {
    if (!event.isTrusted || event.button !== 2 || !contextPress
      || !Number.isFinite(event.timeStamp) || event.timeStamp < contextPress.timeStamp
      || Math.abs(event.clientX - contextPress.x) > 2 || Math.abs(event.clientY - contextPress.y) > 2) return false;
    contextPress = null;
    return true;
  };
  const clear = () => {
    for (const type of types) window.removeEventListener(type, record, true);
    clearTimeout(timer);
    if (window[key]?.token === REQ.receipt) delete window[key];
  };
  const record = event => {
    if (!event.isTrusted) return;
    // Event creation time also catches input queued before the mark but delivered after it.
    if (result.dispatchedAt === null || !Number.isFinite(event.timeStamp)
      || event.timeStamp <= result.dispatchedAt || event.button !== options.button
      || (Number(event.altKey) | Number(event.ctrlKey) * 2 | Number(event.metaKey) * 4 | Number(event.shiftKey) * 8) !== options.modifiers
      || event.type !== expected[result.events.length]) {
      result.ambiguous = true;
      return;
    }
    const path = event.composedPath();
    const actual = path.find(node => node?.nodeType === 1);
    const description = describe(actual);
    if (options.button === 2 && event.type === "mousedown")
      contextPress = { x: event.clientX, y: event.clientY, timeStamp: event.timeStamp };
    result.events.push({ type: event.type, timeStamp: event.timeStamp, matches: Boolean(check?.target && target && path.includes(target)
      && Math.abs(event.clientX - check.x) <= 2 && Math.abs(event.clientY - check.y) <= 2),
    role: hideKeys(description?.role || "element", actual).slice(0, 30),
    name: hideKeys(description?.name || "", actual).slice(0, 160) });
  };
  Object.defineProperty(window, key, { value: { token: REQ.receipt, result, clear, takeContextMenu }, configurable: true });
  for (const type of types) window.addEventListener(type, record, true);
  timer = setTimeout(clear, 6000);
};
