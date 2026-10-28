// A cheap question about the page: for waits and settling after an action. Answers whether `REQ.text`
// is in the visible text, whether `REQ.ref` exists and is visible, the document's ready state, the mutation count
// of the page environment, and drains the dialogs it answered during an agent action.
// REQ: { page, text?, ref?, dialogs? }. Synchronous; the answer is untrusted page data.
/* global ENV, REQ, findRef, visible -- Supplied by the Rust script assembler and its shared prelude. */
(() => {
  __VW_COMMON__
  const out = { url: String(location.href).slice(0, 2000), readyState: document.readyState };
  out.mutations = ENV ? ENV.mutations() : -1;
  if (REQ.text) {
    let text;
    try {
      text = String((document.body && document.body.innerText) || "");
    } catch {
      text = "";
    }
    out.text = text.toLowerCase().includes(String(REQ.text).toLowerCase());
  }
  if (REQ.ref) {
    const found = findRef(REQ.ref);
    out.ref = found.length === 1 && visible(found[0]);
  }
  if (REQ.dialogs && ENV) out.dialogs = ENV.dialogs.splice(0, ENV.dialogs.length);
  return JSON.stringify(out);
})()
