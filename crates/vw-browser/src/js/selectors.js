// Selector matching on an inert, redacted DOM snapshot; observe/extract use original nodes only after matching.
// Counts and ancestor/relative predicates cannot inspect values that the ordinary reader withholds.
/* global COPY_ATTRS, hideKeys, privateField, visible */
/* exported safeSelectors */
const safeSelectors = elements => {
  const copies = new Map(), originals = new Map(), roots = new Map(), privateNodes = new Set();
  const privacy = new Map();
  const protectedField = el => {
    if (!privacy.has(el)) privacy.set(el, privateField(el));
    return privacy.get(el);
  };
  // Other attributes can contain opaque copies of a secret. Keep only ordinary identity,
  // label and state metadata on protected nodes; their values never become queryable data.
  const identityAttributes = new Set("id class name type role autocomplete aria-label aria-labelledby aria-describedby title alt placeholder data-testid data-test-id disabled readonly required multiple checked selected aria-checked aria-selected aria-expanded aria-pressed".split(" "));
  let textBudget = 240000;
  let truncated = false;
  const rootFor = original => {
    if (!roots.has(original)) {
      const doc = document.implementation.createHTMLDocument("");
      doc.replaceChildren();
      roots.set(original, original.nodeType === 9 ? doc : doc.createDocumentFragment());
    }
    return roots.get(original);
  };
  for (const el of elements) {
    const root = rootFor(el.getRootNode());
    const doc = root.nodeType === 9 ? root : root.ownerDocument;
    const copy = doc.createElementNS(el.namespaceURI, el.localName);
    const privateValue = protectedField(el);
    if (privateValue) privateNodes.add(el);
    for (const attr of el.attributes) {
      if (attr.name.startsWith("on") || (privateValue && (COPY_ATTRS.includes(attr.name) || !identityAttributes.has(attr.name)))) continue;
      copy.setAttributeNS(attr.namespaceURI, attr.name, hideKeys(attr.value, el));
    }
    copies.set(el, copy);
    originals.set(copy, el);
    if (privateValue || el.matches("script,style,noscript,template") || !visible(el)) continue;
    // Insert text in its original position once child elements have been copied below.
    for (const child of el.childNodes) {
      if (child.nodeType !== 3) continue;
      const full = hideKeys(child.nodeValue, el);
      const text = full.slice(0, Math.min(2000, textBudget));
      if (text.length < full.length) truncated = true;
      textBudget -= text.length;
      const marker = doc.createTextNode(text);
      copies.set(child, marker);
    }
  }
  for (const el of elements) {
    const copy = copies.get(el);
    for (const child of el.childNodes) {
      const node = copies.get(child);
      if (node) copy.appendChild(node);
    }
  }
  // Priority scans can deliver controls before their ancestors, or retain a late control whose
  // parent is beyond the node cap. Build every copy first, then attach one document root.
  const topLevel = elements.filter(el => !copies.has(el.parentElement)).sort((a, b) =>
    Number(b === b.ownerDocument.documentElement) - Number(a === a.ownerDocument.documentElement));
  for (const el of topLevel) {
    const root = rootFor(el.getRootNode());
    (root.nodeType === 9 && root.documentElement || root).appendChild(copies.get(el));
  }
  // Apply live state only after attachment: inserting options auto-selects the first option in
  // a single select, even when the observed control has selectedIndex=-1. No field value is copied.
  for (const el of elements) {
    const copy = copies.get(el);
    if ("checked" in el) copy.checked = el.checked;
    if (el.localName === "select" && el.options) {
      copy.selectedIndex = -1;
      for (const option of el.options) {
        const copiedOption = copies.get(option);
        if (option.selected && copiedOption) copiedOption.selected = true;
      }
    }
  }
  const query = (kind, value, within = null) => {
    if (kind === "css" && /:(?:visited|hover|active|focus(?:-within|-visible)?)(?![\w-])/i.test(value))
      throw new Error("interaction-state selectors need an observed control state");
    if (within) return originals.get(copies.get(within)?.querySelector(value)) || null;
    const selected = new Set();
    if (kind === "xpath") {
      for (const root of roots.values()) {
        if (root.nodeType !== 9) continue;
        const result = root.evaluate(value, root, null, 7, null);
        for (let i = 0; i < result.snapshotLength; i++) {
          const original = originals.get(result.snapshotItem(i));
          if (original) selected.add(original);
        }
      }
    } else {
      const attribute = {alt:"alt", title:"title", testId:"data-testid"}[kind];
      if (kind !== "css" && !attribute) throw new Error("invalid selector kind");
      for (const el of elements) {
        const copy = copies.get(el);
        if (kind === "css" ? copy.matches(value) : copy.getAttribute(attribute) === value) selected.add(el);
      }
    }
    return selected;
  };
  query.truncated = truncated;
  query.private = el => Boolean(protectedField(el));
  query.kind = el => protectedField(el)?.kind;
  query.label = el => {
    const field = protectedField(el);
    if (field && field.element !== el) return "Private field";
    const copy = copies.get(el), root = el.getRootNode();
    const linked = String(copy?.getAttribute("aria-labelledby") || "").split(/\s+/).filter(Boolean)
      .map(id => query.text(root.getElementById?.(id))).filter(Boolean).join(" ");
    const labels = Array.from(el.labels || [], label => query.text(label)).filter(Boolean).join(" ");
    if (!field) return copy?.getAttribute("aria-label") || linked || labels || null;
    return copy?.getAttribute("aria-label") || linked || labels || copy?.getAttribute("title") || copy?.getAttribute("placeholder") || (field ? "Private field" : null);
  };
  // Text nodes were copied only from rendered, non-secret elements. Separate block boundaries
  // without consulting native form values or falling back to hidden textContent.
  query.text = el => {
    const copy = copies.get(el);
    if (!copy || privateNodes.has(el)) return "";
    const read = node => {
      if (node.nodeType === 3) return node.nodeValue;
      const original = originals.get(node);
      if (!original || privateNodes.has(original)) return "";
      if (original.localName === "br") return "\n";
      const body = Array.from(node.childNodes, read).join("");
      const display = original.ownerDocument.defaultView.getComputedStyle(original).display;
      return /^(block|flex|grid|table|list-item)/.test(display) ? `\n${body}\n` : body;
    };
    return read(copy).replace(/[\t ]+/g, " ").replace(/ *\n */g, "\n").replace(/\n{3,}/g, "\n\n").trim();
  };
  return query;
};
