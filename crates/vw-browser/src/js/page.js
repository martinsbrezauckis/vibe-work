// The whole-page capture's page part: the page as Markdown (a real converter: headings, paragraphs,
// lists, links and images as absolute URLs, code, quotes, tables), as plain visible text (ShellX Browser's
// whole-page text), or as HTML without active content (the live DOM after its scripts ran, with
// scripts, handlers, frames, plugins, `javascript:` URLs, `<base>` and refresh removed; the host adds a `<base>` and
// a CSP that forbids scripts). Password, one-time-code, card and vault-filled fields never leave the
// page. Large answers are handed over in chunks: `start` builds the text once and keeps it in the page environment
// under a token, `chunk` answers a slice, `end` drops it.
// REQ: { page, op: start|chunk|end, format: markdown|text|html, token, offset, maxChars, chunk }.
// Synchronous; the answer is untrusted page data.
(() => {
  __VW_COMMON__
  const head = { url: String(location.href).slice(0, 2000), title: clip(document.title, 300) };
  if (REQ.op === "chunk" || REQ.op === "end") {
    const held = ENV && ENV.captureText;
    if (!held || held.token !== REQ.token) return JSON.stringify({ ...head, error: "changed" });
    if (REQ.op === "end") {
      ENV.captureText = null;
      return JSON.stringify({ ...head, done: true });
    }
    const offset = Math.max(0, Number(REQ.offset) || 0);
    return JSON.stringify({ ...head, chunk: held.text.slice(offset, offset + REQ.chunk), length: held.text.length });
  }
  const absUrl = (value) => {
    try {
      const u = new URL(String(value || ""), location.href);
      return /^(https?|mailto|tel):$/.test(u.protocol) ? u.href : "";
    } catch (_) {
      return "";
    }
  };
  const isHidden = (el) => {
    if (el.getAttribute("aria-hidden") === "true" || el.hidden) return true;
    const cs = viewOf(el).getComputedStyle(el);
    return cs.display === "none" || cs.visibility === "hidden";
  };

  const markdown = () => {
    const SKIP = new Set(["script", "style", "noscript", "template", "head", "svg", "canvas", "iframe", "frame", "object", "embed", "input", "textarea", "select", "button", "option"]);
    const BLOCK = new Set(["address", "article", "aside", "div", "dl", "fieldset", "figure", "footer", "form", "header", "main", "nav", "section", "details", "summary", "dd", "dt", "figcaption"]);
    const esc = (s) => s.replace(/([\\`*_[\]#|])/g, "\\$1");
    const space = (s) => s.replace(/\s+/g, " ");
    let budget = 40000;
    const inline = (node) => {
      if (budget-- <= 0) return "";
      if (node.nodeType === 3) return esc(space(String(node.textContent || "")));
      if (node.nodeType !== 1) return "";
      const el = node;
      const tag = el.localName;
      if (SKIP.has(tag) || isHidden(el)) return "";
      if (tag === "br") return "  \n";
      if (tag === "img") {
        const src = absUrl(el.currentSrc || el.getAttribute("src"));
        return src ? `![${esc(clip(el.getAttribute("alt"), 200))}](${src})` : "";
      }
      const inner = Array.from(el.childNodes).map(inline).join("");
      // Block and flex items laid out side by side (a nav's links) keep a space between them.
      const display = viewOf(el).getComputedStyle(el).display;
      if (!display.startsWith("inline") && tag !== "a") return ` ${inner} `;
      if (tag === "a") {
        if (!display.startsWith("inline")) {
          const href = absUrl(el.getAttribute("href"));
          const text = inner.trim();
          return href && text ? ` [${text}](${href}) ` : ` ${inner} `;
        }
        const href = absUrl(el.getAttribute("href"));
        const text = inner.trim();
        return href && text ? `[${text}](${href})` : inner;
      }
      if ((tag === "strong" || tag === "b") && inner.trim()) return `**${inner.trim()}**`;
      if ((tag === "em" || tag === "i") && inner.trim()) return `_${inner.trim()}_`;
      if (tag === "code") return "`" + String(el.textContent || "").replace(/`/g, "'") + "`";
      return inner;
    };
    const blocks = [];
    const push = (s) => {
      const t = s.replace(/[ \t]+\n/g, "\n").trim();
      if (t) blocks.push(t);
    };
    const table = (el) => {
      const rows = Array.from(el.querySelectorAll("tr")).slice(0, 60);
      if (!rows.length) return;
      const cells = rows.map((tr) => Array.from(tr.children).slice(0, 12).map((c) => inline(c).replace(/\n/g, " ").trim()));
      const width = Math.max(...cells.map((r) => r.length));
      const line = (r) => `| ${Array.from({ length: width }, (_, i) => r[i] || "").join(" | ")} |`;
      push([line(cells[0]), `| ${Array.from({ length: width }, () => "---").join(" | ")} |`, ...cells.slice(1).map(line)].join("\n"));
    };
    const list = (el, depth) => {
      const ordered = el.localName === "ol";
      let n = 0;
      const lines = [];
      for (const li of Array.from(el.children)) {
        if (li.localName !== "li" || isHidden(li)) continue;
        n += 1;
        const own = Array.from(li.childNodes).filter((c) => !(c.nodeType === 1 && (c.localName === "ul" || c.localName === "ol"))).map(inline).join("").trim();
        lines.push(`${"  ".repeat(depth)}${ordered ? `${n}.` : "-"} ${own}`);
        for (const sub of Array.from(li.children)) if (sub.localName === "ul" || sub.localName === "ol") lines.push(list(sub, depth + 1));
      }
      return lines.filter(Boolean).join("\n");
    };
    const block = (node) => {
      if (budget-- <= 0) return;
      if (node.nodeType === 3) {
        push(inline(node));
        return;
      }
      if (node.nodeType !== 1) return;
      const el = node;
      const tag = el.localName;
      if (SKIP.has(tag) || isHidden(el)) return;
      const h = /^h([1-6])$/.exec(tag);
      if (h) return push(`${"#".repeat(Number(h[1]))} ${inline(el).trim()}`);
      if (tag === "p") return push(inline(el));
      if (tag === "ul" || tag === "ol") return push(list(el, 0));
      if (tag === "pre") return push("```\n" + String(el.textContent || "").replace(/```/g, "'''").replace(/\n+$/, "") + "\n```");
      if (tag === "blockquote") return push(inline(el).trim().split("\n").map((l) => `> ${l}`).join("\n"));
      if (tag === "table") return table(el);
      if (tag === "hr") return push("---");
      if (tag === "img") return push(inline(el));
      // A block whose children are all inline is one paragraph; otherwise its children are blocks.
      const hasBlock = Array.from(el.children).some((c) => BLOCK.has(c.localName) || /^(h[1-6]|p|ul|ol|pre|blockquote|table|hr|li)$/.test(c.localName));
      if (!hasBlock && (BLOCK.has(tag) || tag === "li" || tag === "body")) return push(inline(el));
      for (const child of Array.from(el.childNodes)) block(child);
    };
    if (document.body) block(document.body);
    return `# ${esc(clip(document.title, 300)) || "Untitled page"}\n\n${blocks.join("\n\n")}\n`;
  };

  const plainText = () => {
    let text = "";
    try {
      text = String((document.body && document.body.innerText) || "");
    } catch (_) {
      text = "";
    }
    return text;
  };

  const html = () => {
    const root = document.documentElement;
    const copy = root.cloneNode(true);
    // Walk the original and the copy together (same shape) to blank secret values.
    const a = document.createTreeWalker(root, 1);
    const b = document.createTreeWalker(copy, 1);
    let x = a.currentNode, y = b.currentNode;
    for (let steps = 0; x && y && steps < 200000; steps += 1) {
      const tag = x.localName;
      if (tag === "input" || tag === "textarea") {
        if (secretKind(x) || vaultFilled(x)) {
          y.removeAttribute("value");
          if (tag === "textarea") y.textContent = "";
        }
      }
      x = a.nextNode();
      y = b.nextNode();
    }
    const DROP = "script,noscript,iframe,frame,frameset,object,embed,applet,base,portal,meta[http-equiv],link[rel~=preload],link[rel~=modulepreload],link[rel~=prefetch],link[rel~=import],link[rel~=manifest],link[rel~=serviceworker]";
    for (const el of Array.from(copy.querySelectorAll(DROP))) el.remove();
    const URL_ATTRS = ["href", "src", "action", "formaction", "poster", "xlink:href", "data", "cite", "background", "ping"];
    for (const el of Array.from(copy.querySelectorAll("*"))) {
      for (const attr of Array.from(el.attributes)) {
        const name = attr.name.toLowerCase();
        if (name.startsWith("on") || name === "srcdoc" || name === "nonce" || name === "integrity") {
          el.removeAttribute(attr.name);
          continue;
        }
        if (URL_ATTRS.includes(name)) {
          const value = String(attr.value || "").trim();
          if (/^\s*(javascript|vbscript|data:text\/html|data:application)/i.test(value)) el.removeAttribute(attr.name);
          else if (value && !value.startsWith("#")) {
            try {
              el.setAttribute(attr.name, new URL(value, location.href).href);
            } catch (_) {
              el.removeAttribute(attr.name);
            }
          }
        }
        if (name === "srcset") el.removeAttribute(attr.name);
      }
    }
    return "<!DOCTYPE html>\n" + copy.outerHTML;
  };

  let text = REQ.format === "markdown" ? markdown() : REQ.format === "html" ? html() : plainText();
  // Control characters other than tab and line ends would bloat the JSON answer (\u escapes): dropped.
  text = text.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "");
  const cut = text.length > REQ.maxChars;
  if (cut) text = text.slice(0, REQ.maxChars);
  if (ENV) ENV.captureText = { token: REQ.token, text };
  return JSON.stringify({ ...head, length: text.length, cut, chunk: text.slice(0, REQ.chunk), held: Boolean(ENV) });
})()
