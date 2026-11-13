// Bounded inspection of exactly one element in the current main document, evaluated in the isolated agent world.
// The host supplies JSON only. Markup excludes scripts, event handlers, hidden inputs and form values.
(() => {
  // eslint-disable-next-line no-undef -- JSON request token supplied by the Rust assembler.
  const req = __REQUEST__;
  const fail = error => JSON.stringify({ error });
  if (location.href !== req.url) return fail("The page changed; read it again before inspecting an element.");
  let matches;
  try { matches = document.querySelectorAll(req.selector); }
  catch { return fail("The selector is invalid; use a CSS selector for one element."); }
  if (matches.length !== 1) return fail(`The selector matched ${matches.length} elements; use a selector for exactly one element.`);
  const element = matches[0];
  const sensitive = el => /^(script|template)$/i.test(el.localName) || el.matches('input[type="password"],input[type="hidden"],[autocomplete^="cc-"]');
  const privateName = name => /(?:password|secret|token|authorization|cookie|nonce|integrity)/i.test(name) || /^on/i.test(name);
  if (sensitive(element)) return fail("This element may contain hidden credentials; inspect a visible non-secret element instead.");
  if (req.attribute && (privateName(req.attribute) || req.attribute.toLowerCase() === "value")) return fail("This attribute may contain a credential or field value; read the visible page instead.");
  const copy = document.createElement("div");
  let count = 0, limited = false;
  const visit = (node, parent, depth) => {
    if (++count > 200 || depth > 16) { limited = true; return; }
    if (node.nodeType === Node.TEXT_NODE) { parent.appendChild(document.createTextNode(node.textContent.slice(0, 1200))); return; }
    if (node.nodeType !== Node.ELEMENT_NODE || sensitive(node)) return;
    const cloned = node.cloneNode(false);
    for (const attr of Array.from(cloned.attributes)) {
      if (privateName(attr.name) || attr.name === "value" || attr.name === "srcdoc" || attr.value.length > 512) cloned.removeAttribute(attr.name);
    }
    parent.appendChild(cloned);
    if (node.matches("textarea,[contenteditable]")) return;
    for (const child of node.childNodes) { visit(child, cloned, depth + 1); if (count >= 200) { limited = true; break; } }
  };
  visit(element, copy, 0);
  const html = copy.innerHTML;
  const computed = getComputedStyle(element);
  const names = req.properties || ["display","position","width","height","color","background-color","font-family","font-size","font-weight","margin","padding","border","gap"];
  const styles = Object.create(null);
  for (const name of names) styles[name] = computed.getPropertyValue(name).slice(0, 128);
  const box = element.getBoundingClientRect();
  return JSON.stringify({
    tag: element.localName, html: html.slice(0, 1200), htmlTruncated: limited || html.length > 1200,
    sanitized: true, styles, box: { x: box.x, y: box.y, width: box.width, height: box.height },
    ...(req.attribute ? { attribute: { name: req.attribute, value: element.getAttribute(req.attribute)?.slice(0, 512) ?? null } } : {}),
  });
})()
