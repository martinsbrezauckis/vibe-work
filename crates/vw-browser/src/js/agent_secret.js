// Captures one retained page ref for the native vault broker. No page exception or observation is returned.
// Depends on the isolated common prelude; called only by the optional agent vault module.
/* global ENV, REQ, collect, refOf, observedTarget, visible */
(() => {
  __VW_COMMON__
  const reply = (status, value) => JSON.stringify({status, url: String(location.href), ...(value === undefined ? {} : {value})});
  try {
    if (!ENV) return reply("stale");
    const targets = collect().elements.filter(el => refOf(el) === REQ.ref && visible(el)
      && observedTarget(el, REQ.observedNode, REQ.observedName));
    if (targets.length !== 1) return reply("stale");
    const target = targets[0];
    // Read exactly the pointed input or text, never a label, attribute, sibling or surrounding page.
    const value = target.localName === "input" || target.localName === "textarea"
      ? String(target.value) : String(target.innerText || target.textContent || "");
    if (!value || new TextEncoder().encode(value).length > 4096) return reply("invalid");
    return reply("captured", value);
  } catch { return reply("unreadable"); }
})()
