// Event-time target guard and evidence for point.js, held only in the agent's isolated frame world.
// The document-start capture boundary blocks a changed target before page handlers and default activation.
/* global REQ, describe, hideKeys -- Supplied by the point script assembler. */
/* exported armPointReceipt */
const armPointReceipt = (check, target) => {
  const key = REQ.page + "_pointReceipt";
  window[key]?.clear();
  const expected = REQ.expected || ["mousedown", "mouseup", "click"];
  const types = [...new Set(expected)];
  const options = REQ.click || { button: 0, count: 1, modifiers: 0 };
  const result = { token: REQ.receipt, document: window[REQ.page]?.frameDocument,
    dispatchedAt: null, ambiguous: false, blocked: null, events: [] };
  const boundary = window.__vwPointGuard;
  if (!boundary) throw new Error("checked input capture is unavailable");
  // A label's native default forwards a trusted click to its associated control, possibly outside its subtree.
  // Freeze that identity before input; a page changing the association must not gain a second destination.
  let label = target;
  while (label && label.localName !== "label") label = label.assignedSlot || label.parentElement || label.getRootNode()?.host;
  const control = label?.control;
  let labelPress = null;
  const forwarded = new WeakSet();
  const sameAssociation = () => {
    try { return !label || label.control === control && (!control || control.isConnected); }
    catch { return false; }
  };
  const hitTarget = () => {
    if (!check || !target?.isConnected) return false;
    let hit = document.elementFromPoint(check.x, check.y);
    for (let i = 0; hit?.shadowRoot && i < 8; i += 1) {
      const next = hit.shadowRoot.elementFromPoint(check.x, check.y);
      if (!next || next === hit) break;
      hit = next;
    }
    // An iframe is authorized only as an ancestor of the validated target's own document.
    if (["iframe", "frame"].includes(hit?.localName) && (check.target || hit !== target)) return false;
    for (let node = hit; node; node = node.parentElement || node.getRootNode()?.host)
      if (node === target) return true;
    return false;
  };
  const mark = () => {
    if (result.dispatchedAt !== null) return false;
    if (check && !hitTarget()) return false;
    result.dispatchedAt = performance.now();
    return true;
  };
  const guard = event => {
    // Pointer defaults may produce input events on an associated control; keyboard receipts own those guards.
    if (["keydown", "keyup", "beforeinput", "input"].includes(event.type)) return;
    if (!event.isTrusted || options.count === 0 && !options.wheel || result.dispatchedAt === null) return;
    if (event.type === "pointerdown" || event.type === "mousedown") labelPress = null;
    const path = event.composedPath();
    const forwarding = labelPress && event.type === "click" && event.button === 0;
    const allowedForward = forwarding && control && path.includes(control)
      && event.clientX === labelPress.x && event.clientY === labelPress.y;
    // Frame motion, clock rounding and native compatibility-event fields cannot identify the input source.
    // The marked interval always enforces target identity; fields and timing qualify the receipt below.
    if (result.blocked || !check?.target || !sameAssociation() || !hitTarget()
      || (forwarding ? !allowedForward : !path.includes(target))) {
      // Enforce first: collecting a diagnostic must never let an event through if a DOM read fails.
      event.preventDefault();
      event.stopImmediatePropagation();
      const first = !result.blocked;
      result.blocked ||= { role: "element", name: "" };
      const actual = event.composedPath().find(node => node?.nodeType === 1);
      const description = describe(actual);
      if (first) result.blocked = { role: hideKeys(description?.role || "element", actual).slice(0, 30),
        name: hideKeys(description?.name || "", actual).slice(0, 160) };
    } else if (allowedForward) {
      labelPress = null;
      forwarded.add(event);
    } else if (label && control && event.type === "click" && event.button === 0) {
      labelPress = { x: event.clientX, y: event.clientY };
    }
    // A person's interleaved sequence cannot consume protection before queued agent input arrives.
    // Keep enforcement until the host collects/clears this receipt, including trailing compatibility events.
  };
  boundary.set(guard);
  let scrollNode = target;
  if (options.wheel) {
    for (; scrollNode; scrollNode = scrollNode.parentElement || scrollNode.getRootNode()?.host) {
      const style = getComputedStyle(scrollNode);
      if ((/auto|scroll/.test(style.overflowX) && scrollNode.scrollWidth > scrollNode.clientWidth)
        || (/auto|scroll/.test(style.overflowY) && scrollNode.scrollHeight > scrollNode.clientHeight)) break;
    }
    scrollNode ||= document.scrollingElement;
  }
  const position = () => ({ x:scrollNode?.scrollLeft ?? 0, y:scrollNode?.scrollTop ?? 0, documentX:scrollX, documentY:scrollY });
  const before = options.wheel ? position() : null;
  let last = before, changedAt = null;
  const read = () => {
    if (options.wheel && check?.target && result.events.length) {
      const current = position();
      if (!last || current.x !== last.x || current.y !== last.y || current.documentX !== last.documentX || current.documentY !== last.documentY || changedAt === null) changedAt = performance.now();
      last = current;
      result.scroll = { ...current, moved:current.x !== before.x || current.y !== before.y,
        connected:Boolean(scrollNode?.isConnected), deltaX:options.wheel[0], deltaY:options.wheel[1] };
      result.ready = performance.now() - changedAt >= 160;
    }
    return result;
  };
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
    boundary.clear(guard);
    for (const type of types) window.removeEventListener(type, record, true);
    clearTimeout(timer);
    if (window[key]?.token === REQ.receipt) delete window[key];
  };
  const record = event => {
    if (!event.isTrusted || forwarded.has(event)) return;
    // Event creation time also catches input queued before the mark but delivered after it.
    if (result.dispatchedAt === null || !Number.isFinite(event.timeStamp)
      || event.timeStamp <= result.dispatchedAt || event.button !== options.button
      || (Number(event.altKey) | Number(event.ctrlKey) * 2 | Number(event.metaKey) * 4 | Number(event.shiftKey) * 8) !== options.modifiers
      || event.type !== expected[result.events.length]
      || options.wheel && (event.deltaMode !== 0 || Math.abs(event.deltaX - options.wheel[0]) > 1 || Math.abs(event.deltaY - options.wheel[1]) > 1)) {
      result.ambiguous = true;
      return;
    }
    const path = event.composedPath();
    const actual = path.find(node => node?.nodeType === 1);
    const description = describe(actual);
    if (options.button === 2 && event.type === "mousedown")
      contextPress = { x: event.clientX, y: event.clientY, timeStamp: event.timeStamp };
    const matches = Boolean(check?.target && target && path.includes(target)
      && Math.abs(event.clientX - check.x) <= 2 && Math.abs(event.clientY - check.y) <= 2);
    // Windows middle-button autoscroll consumes auxclick and leaves a pointer mode behind. Cancel only this
    // checked press's default, so the page still receives its auxclick (including a link's normal default).
    if (matches && options.button === 1 && event.type === "mousedown") event.preventDefault();
    result.events.push({ type: event.type, timeStamp: event.timeStamp, matches,
    role: hideKeys(description?.role || "element", actual).slice(0, 30),
    name: hideKeys(description?.name || "", actual).slice(0, 160) });
  };
  Object.defineProperty(window, key, { value: { token: REQ.receipt, result, read, clear, mark, takeContextMenu }, configurable: true });
  for (const type of types) window.addEventListener(type, record, true);
  timer = setTimeout(clear, 6000);
};
