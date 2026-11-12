// Rendered headings and terms around choices, outside find/text-page quotas. Included by scripts.rs.
// Depends on the observation's rendered choice groups and common redaction helpers; never interprets offers.
/* global hideKeys, visible, viewOf */
/* exported choiceContextFor, overlayChoiceRoots */
const choiceRenderedText = (root, max, privateNode) => {
  const parts = [];
  let length = 0, count = 0, complete = true;
  const walker = root.ownerDocument.createTreeWalker(root, 4);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (++count > 12000) { complete = false; break; }
    const parent = node.parentElement;
    if (!parent || privateNode?.(parent) || parent.closest("script,style,noscript,template") || !visible(parent)) continue;
    const text = hideKeys(String(node.textContent || "").trim(), parent);
    if (!text) continue;
    const separator = Number(parts.length > 0), room = Math.max(0, max - length - separator);
    if (text.length > room) {
      if (room) parts.push(text.slice(0, room));
      complete = false; break;
    }
    parts.push(text);
    length += text.length + separator;
  }
  return { text: parts.join("\n"), complete };
};

const choiceContextFor = (root, privateNode) => {
  const headings = [];
  const body = choiceRenderedText(root, 1000, privateNode);
  let complete = body.complete;
  for (const heading of root.querySelectorAll('h1,h2,h3,h4,h5,h6,[role="heading"]')) {
    if (!visible(heading)) continue;
    const rendered = choiceRenderedText(heading, 180, privateNode);
    if (!rendered.text) continue;
    if (headings.length === 8) { complete = false; break; }
    headings.push(rendered.text);
    complete &&= rendered.complete;
  }
  return { frameOrigin: hideKeys(root.ownerDocument.location.origin), headings, text: body.text, complete };
};

const overlayChoiceRoots = choices => {
  const roots = new Set();
  for (const { el } of choices) {
    let root = null;
    for (let up = el; up && up !== el.ownerDocument.body; up = up.parentElement || up.getRootNode()?.host) {
      if (up.matches('[role="dialog"],[aria-modal="true"],dialog[open]')) root = up;
      else if (!root) {
        const position = viewOf(up).getComputedStyle(up).position;
        if (up.getAttribute("role") === "region" && ["fixed", "sticky"].includes(position)) root = up;
        else if (position === "fixed") {
          const r = up.getBoundingClientRect(), win = viewOf(up);
          const width = Math.max(0, Math.min(r.right, win.innerWidth) - Math.max(0, r.left));
          const height = Math.max(0, Math.min(r.bottom, win.innerHeight) - Math.max(0, r.top));
          if (width * height >= win.innerWidth * win.innerHeight * 0.4) root = up;
        }
      }
    }
    if (root) roots.add(root);
  }
  return [...roots].filter(root => ![...roots].some(other => other !== root && other.contains(root)));
};
