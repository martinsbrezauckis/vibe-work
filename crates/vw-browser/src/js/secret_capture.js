// Captures only a physical page selection or click for a user-initiated vault save.
// The answer goes to the native browser core, never to the UI or an agent tool.
// REQ: { page, mode: selection|element|contextSelection, op: start|poll|cancel }.
/* global COPY_ATTRS, ENV, REQ, keyTokens -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  __VW_COMMON__
  const NAME = "__vw_secret_choice";
  const reply = (status, extra = {}) => JSON.stringify({
    status, url: String(location.href).slice(0, 2000),
    documentId: ENV && (ENV.secretDocumentId ||= globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random()}`),
    ...extra,
  });
  if (!ENV) return reply("stale");
  if (REQ.op === "document") return reply("document");
  const prior = window[NAME];
  if (REQ.op === "poll") return prior ? prior.read() : reply("stale");
  if (REQ.op === "cancel") {
    if (prior) prior.stop();
    return reply("cancelled");
  }
  if (prior) {
    prior.stop();
    delete window[NAME];
  }
  const finish = (value, element) => {
    value = String(value || "").trim();
    if (!value) return reply("empty");
    if (value.length > 4096) return reply("tooLong");
    return reply("captured", { value, element });
  };
  if (REQ.mode === "selection") {
    const chosen = window.getSelection();
    return finish(chosen && chosen.toString(), "selected text");
  }
  if (REQ.mode === "contextSelection") {
    const choice = ENV.takeContextChoice && ENV.takeContextChoice();
    if (choice?.tokenize) {
      const tokens = keyTokens(choice.value, choice.target);
      if (tokens.length > 1) return reply("multipleValues");
      if (tokens.length === 1) choice.value = tokens[0];
    }
    return finish(choice?.value, choice?.element || "selected text");
  }
  if (REQ.mode !== "element") return reply("stale");
  const outline = document.createElement("div");
  outline.style.cssText = "position:fixed;pointer-events:none;z-index:2147483647;outline:2px solid #fa500f;display:none";
  document.documentElement.appendChild(outline);
  let answer = reply("pending");
  const move = (event) => {
    const rect = event.target && event.target.getBoundingClientRect && event.target.getBoundingClientRect();
    if (!rect) return;
    outline.style.display = "block";
    outline.style.left = rect.left + "px";
    outline.style.top = rect.top + "px";
    outline.style.width = rect.width + "px";
    outline.style.height = rect.height + "px";
  };
  const stop = () => {
    window.removeEventListener("mousemove", move, true);
    window.removeEventListener("click", click, true);
    window.removeEventListener("keydown", key, true);
    outline.remove();
    if (answer.includes('"pending"')) answer = reply("cancelled");
  };
  const click = (event) => {
    if (!event.isTrusted) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    const el = event.target;
    let value = "";
    if (el.localName === "input" || el.localName === "textarea") value = String(el.value || "");
    if (!value.trim()) {
      for (const attr of COPY_ATTRS) {
        const candidate = el.getAttribute && el.getAttribute(attr);
        if (candidate && candidate.trim()) { value = candidate; break; }
      }
    }
    if (!value.trim()) {
      value = String(el.innerText || el.textContent || "");
      const tokens = keyTokens(value, el);
      if (tokens.length > 1) {
        answer = reply("multipleValues");
        stop();
        return;
      }
      if (tokens.length === 1) value = tokens[0];
    }
    answer = finish(value, String(el.localName || "element").slice(0, 30));
    stop();
  };
  const key = (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      answer = reply("cancelled");
      stop();
    }
  };
  window.addEventListener("mousemove", move, true);
  window.addEventListener("click", click, true);
  window.addEventListener("keydown", key, true);
  const read = () => {
    const result = answer;
    if (!result.includes('"pending"')) {
      answer = reply("stale");
      stop();
    }
    return result;
  };
  Object.defineProperty(window, NAME, { value: { read, stop }, configurable: true });
  return answer;
})()
