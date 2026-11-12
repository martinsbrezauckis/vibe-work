/** Lazy, DOM-free Mermaid rendering. Called by MermaidBlock; only passiveMarkup's output reaches the page. */
import { parseMermaid, renderMermaid } from "beautiful-mermaid";

// The small upstream parser accepts only newline-separated graph statements and can silently
// truncate unfinished shapes. Normalize separators without touching labels, and reject truncation.
function graphSource(source: string): string {
  let out = "";
  let quoted = false;
  let escaped = false;
  const brackets: string[] = [];
  for (const character of source) {
    if (escaped) escaped = false;
    else if (quoted && character === "\\") escaped = true;
    else if (character === '"') quoted = !quoted;
    else if (!quoted) {
      if ("[({".includes(character)) brackets.push(character);
      else if ("])}".includes(character) && brackets.length > 0) {
        const expected = { "]": "[", ")": "(", "}": "{" }[character];
        if (brackets.pop() !== expected) throw new Error("Unfinished shape");
      }
      if (character === ";" && brackets.length === 0) {
        out += "\n";
        continue;
      }
    }
    out += character;
  }
  if (quoted || brackets.length > 0 || /(?:-->|---|==>|-\.->)\s*$/m.test(out)) {
    throw new Error("Unfinished diagram");
  }
  return out;
}

/** Bound expensive layout and refuse configuration/styling directives; diagrams inherit the app's theme. */
export async function drawDiagram(source: string): Promise<string> {
  if (source.length > 8000) throw new Error("Diagram too large");
  const lines = source
    .split("\n")
    .filter((line) => !line.trim().startsWith("%%"))
    .join("\n");
  const graphKind = /^\s*(?:graph|flowchart|stateDiagram)/i.test(lines);
  const input = graphKind ? graphSource(lines) : source;
  if (
    input.split("\n").length > 160 ||
    /%%\{|^\s*---|^\s*(?:click|style|classDef|linkStyle)\s/im.test(input) ||
    /%%\{/.test(source)
  ) {
    throw new Error("Unsupported diagram");
  }
  if (graphKind) {
    const graph = parseMermaid(input);
    if (graph.nodes.size === 0 || graph.nodes.size > 120 || graph.edges.length > 240) {
      throw new Error("Diagram too large or empty");
    }
  }
  return renderMermaid(input, { transparent: true, font: "sans-serif", padding: 20 });
}
