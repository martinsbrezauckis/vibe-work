// Event-time keyboard target proof in the isolated world; point.js arms it after exact frame validation.
// Native adapters supply trusted events; the host collects this receipt once and never retries sent input.
/* global REQ, editable, secretKind, implicitSubmission -- Supplied by the point script assembler. */
/* exported armKeyboardReceipt */
const armKeyboardReceipt = (check, target) => {
  const key = REQ.page + "_pointReceipt";
  window[key]?.clear();
  const input = REQ.keyboard;
  const text = typeof input.text === "string";
  const expected = text ? ["beforeinput", "input"] : ["keydown", "keyup"];
  const result = { token: REQ.receipt, document: window[REQ.page]?.frameDocument,
    dispatchedAt: null, ambiguous: false, blocked: null, events: [] };
  const boundary = window.__vwPointGuard;
  if (!boundary) throw new Error("checked input capture is unavailable");
  const active = () => { let node = document.activeElement; while (node?.shadowRoot?.activeElement) node = node.shadowRoot.activeElement; return node; };
  const focused = () => Boolean(check?.target && target?.isConnected && active() === target &&
    (!input.editor || editable(target)) && (!(text || input.replace || input.writes || input.editor) || !secretKind(target)));
  if (check?.target) {
    if (!target.ownerDocument.hasFocus()) target.ownerDocument.defaultView.focus();
    if (active() !== target) target.focus({ preventScroll: true });
    if (!focused()) return false;
    if (input.replace) {
      if (target.isContentEditable) {
        const range = document.createRange(); range.selectNodeContents(target);
        const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
      } else target.select();
    }
  }
  if (check?.target && input.insert) {
    let expected;
    if (target.isContentEditable) {
      const selection = getSelection();
      let range = selection.rangeCount ? selection.getRangeAt(0).cloneRange() : document.createRange();
      if (!target.contains(range.startContainer) || !target.contains(range.endContainer)) {
        range.selectNodeContents(target); range.collapse(false);
        selection.removeAllRanges(); selection.addRange(range);
      }
      const before = document.createRange(), after = document.createRange();
      before.selectNodeContents(target); before.setEnd(range.startContainer, range.startOffset);
      after.selectNodeContents(target); after.setStart(range.endContainer, range.endOffset);
      expected = before.toString() + input.text + after.toString();
    } else {
      if (typeof target.selectionStart !== "number") return false;
      expected = target.value.slice(0, target.selectionStart) + input.text + target.value.slice(target.selectionEnd);
    }
    const env = window[REQ.page]; env.insertExpected ||= new WeakMap(); env.insertExpected.set(target, expected);
  }
  const previous = text && check?.target ? String(target.isContentEditable ? target.textContent : target.value) : null;
  const mark = () => {
    if (result.dispatchedAt !== null || check?.target && !focused()) return false;
    result.dispatchedAt = performance.now();
    return true;
  };
  const submission = check?.target && input.key === "Enter" ? implicitSubmission(target) : null;
  const form = submission?.form || (check?.target && input.key === "Enter" ? target.form : null);
  let invalid = false, submitted = null, forwarded = false;
  const rejected = event => { if (event.isTrusted && event.target?.form === form) invalid = true; };
  const submittedEvent = event => { if (event.isTrusted && event.target === form) submitted = event; };
  if (form) { window.addEventListener("invalid", rejected, true); window.addEventListener("submit", submittedEvent, true); }
  const modifiers = event => [event.ctrlKey && "Control", event.shiftKey && "Shift", event.altKey && "Alt", event.metaKey && "Meta"].filter(Boolean).sort().join(",");
  let inputAt = null, inputs = 0, beforeInput = null, beforeAt = null;
  const guard = event => {
    if (!event.isTrusted || result.dispatchedAt === null) return;
    const path = event.composedPath();
    const tabRelease = !text && input.key === "Tab" && event.type === "keyup" && result.events[0]?.type === "keydown";
    const current = submission && implicitSubmission(target);
    const activation = !forwarded && event.type === "click" && event.detail === 0 && result.events[0]?.type === "keydown" &&
      submission?.submitter && current?.form === form && current.submitter === submission.submitter && path.includes(submission.submitter);
    const validationRelease = input.key === "Enter" && event.type === "keyup" && invalid && event.target?.form === form && result.events[0]?.type === "keydown";
    const same = tabRelease || validationRelease || focused() && (path.includes(target) || activation);
    if (activation) forwarded = true;
    let valid = same;
    const textChanged = event.type === "keydown" && typeof input.expectedText === "string" &&
      String(target?.isContentEditable ? target.textContent : target?.value) !== input.expectedText;
    valid &&= !textChanged;
    if (event.type === "keydown" || event.type === "keyup")
      valid &&= !text && event.key === (input.key === "Space" ? " " : input.key) && modifiers(event) === [...(input.modifiers || [])].sort().join(",");
    else if (event.type === "beforeinput")
      valid &&= !secretKind(target) && (!text || event.data === input.text && event.inputType === "insertText");
    else if (event.type === "input")
      valid &&= !text || result.events[0]?.type === "beforeinput" && ["insertText", "insertLineBreak", "insertParagraph"].includes(event.inputType);
    else valid &&= !text && ["Enter", "Space"].includes(input.key) && event.type === "click" && event.detail === 0;
    if (text && event.type === "beforeinput")
      valid &&= String(target?.isContentEditable ? target.textContent : target?.value) === previous;
    if (result.blocked || !valid) {
      event.preventDefault(); event.stopImmediatePropagation();
      result.blocked ||= { role: "keyboard target", name: textChanged ? "requested text changed" : "focus, content or input sequence changed" };
    }
    if (event.type === "beforeinput") { beforeInput = event; beforeAt = performance.now(); }
    if (event.type === "input") inputAt = performance.now();
    if (!expected.includes(event.type)) return;
    // WebKit reports one beforeinput for a multiline insertion, then several input events for its paragraphs.
    // Only that admitted insertion may extend the input receipt; another beforeinput/key is still ambiguous.
    const continuation = text && event.type === "input" && result.events.length === 2;
    if ((!continuation && event.type !== expected[result.events.length]) || !Number.isFinite(event.timeStamp) || event.timeStamp <= result.dispatchedAt) {
      result.ambiguous = true; return;
    }
    if (text && event.type === "input") {
      inputs += 1;
      if (inputs > input.text.length * 2 + 2) result.ambiguous = true;
    }
    const receipt = { type: event.type, timeStamp: event.timeStamp, matches: valid };
    if (continuation) result.events[1] = receipt;
    else result.events.push(receipt);
  };
  const changes = new MutationObserver(() => {
    if (beforeInput && inputAt === null && (!focused() || secretKind(target))) {
      beforeInput.preventDefault();
      result.blocked ||= { role: "keyboard target", name: "target became private or changed before text entry" };
    }
  });
  changes.observe(document, { subtree: true, attributes: true, childList: true, characterData: true });
  boundary.set(guard);
  const clear = () => {
    boundary.clear(guard); changes.disconnect();
    window.removeEventListener("invalid", rejected, true); window.removeEventListener("submit", submittedEvent, true);
    if (window[key]?.token === REQ.receipt) delete window[key];
  };
  const read = () => {
    if (text) {
      result.rejected = Boolean(beforeInput?.defaultPrevented && inputAt === null);
      result.ready = (inputAt !== null && performance.now() - inputAt >= 30) ||
        (result.rejected && performance.now() - beforeAt >= 30);
    }
    if (invalid) result.effect = "Native validation blocked submission. Read the invalid field messages and correct the fields before submitting again.";
    else if (submitted?.defaultPrevented) result.effect = "The page handled and canceled the native submit event. Read the resulting page state to confirm its outcome.";
    else if (submitted) result.effect = "The native submit event ran. Read the resulting page to confirm the submission outcome.";
    return result;
  };
  Object.defineProperty(window, key, { value: { token: REQ.receipt, result, read, clear, mark }, configurable: true });
  return true;
};
