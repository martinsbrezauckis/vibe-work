// Inspect one live authorization document before an automated action. The returned snapshot is repeated inside
// act.js after a native dialog so a changed page, permission set, or target cannot inherit the earlier answer.
(() => {
  __VW_COMMON__
  if (!ENV) return JSON.stringify({ url: String(location.href), stale: true });
  return JSON.stringify(consentSnapshot(REQ.ref || ""));
})()
