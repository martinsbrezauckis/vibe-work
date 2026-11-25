/** First gate of every diagram: size bound, refused directives, comment stripping and header detection. */
import { MAX_LINE_CHARS, MAX_SOURCE_CHARS } from "./limits";
import {
  DiagramInputError,
  DiagramLimitError,
  DiagramUnsupportedError,
  type DiagramGroup,
  type DiagramType,
  type PreparedSource,
} from "./types";

// Header word(s) of each family as Mermaid 12.3.0 documents them (the beta spellings included; checked against the
// saved syntax pages). The order matters only where one header is a prefix of another; each pattern is anchored.
const headers: [DiagramType, RegExp][] = [
  ["flowchart", /^(?:flowchart(?:-v2|-elk)?|graph)(?:\s|$)/i],
  ["sequence", /^sequenceDiagram\s*$/i],
  ["class", /^classDiagram(?:-v2)?\s*$/i],
  ["state", /^stateDiagram(?:-v2)?\s*$/i],
  ["er", /^erDiagram\s*$/i],
  ["pie", /^pie(?:\s|$)/i],
  ["gantt", /^gantt\s*$/i],
  ["quadrant", /^quadrantChart\s*$/i],
  ["xychart", /^xychart(?:-beta)?(?:\s|$)/i],
  ["sankey", /^sankey(?:-beta)?\s*$/i],
  ["radar", /^radar(?:-beta)?\s*$/i],
  ["journey", /^journey\s*$/i],
  ["timeline", /^timeline\s*$/i],
  ["gitgraph", /^gitGraph(?:\s|:|$)/i],
  ["kanban", /^kanban\s*$/i],
  ["c4", /^C4(?:Context|Container|Component|Dynamic|Deployment)\s*$/i],
  ["requirement", /^requirementDiagram\s*$/i],
  ["architecture", /^architecture(?:-beta)?\s*$/i],
  ["block", /^block(?:-beta)?\s*$/i],
  ["packet", /^packet(?:-beta)?\s*$/i],
  ["mindmap", /^mindmap\s*$/i],
  ["treemap", /^treemap(?:-beta)?\s*$/i],
  ["zenuml", /^zenuml\s*$/i],
  ["swimlane", /^swimlane-beta\s*$/i],
  ["usecase", /^usecase-beta\s*$/i],
  ["eventmodeling", /^eventmodeling\s*$/i],
  ["ishikawa", /^ishikawa-beta\s*$/i],
  ["treeview", /^treeView-beta\s*$/i],
  ["agentflow", /^agentflow(?:-beta)?\s*$/i],
  ["venn", /^venn-beta\s*$/i],
  ["wardley", /^wardley-beta\s*$/i],
  ["cynefin", /^cynefin-beta\s*$/i],
];

/** Which registry slot owns each family. The group files hold the families; this map is fixed. */
export const GROUP_OF: Record<DiagramType, DiagramGroup> = {
  flowchart: "core",
  sequence: "core",
  class: "core",
  state: "core",
  er: "core",
  pie: "core",
  gantt: "core",
  quadrant: "charts",
  xychart: "charts",
  sankey: "charts",
  radar: "charts",
  journey: "timelines",
  timeline: "timelines",
  gitgraph: "timelines",
  kanban: "timelines",
  c4: "architecture",
  requirement: "architecture",
  architecture: "architecture",
  block: "architecture",
  packet: "architecture",
  mindmap: "hierarchy",
  treemap: "hierarchy",
  zenuml: "specialty",
  swimlane: "specialty",
  usecase: "specialty",
  eventmodeling: "specialty",
  ishikawa: "hierarchy",
  treeview: "hierarchy",
  agentflow: "specialty",
  venn: "hierarchy",
  wardley: "specialty",
  cynefin: "specialty",
};

/** Detect the family from the first meaningful line, or return undefined. */
export function detectType(header: string): DiagramType | undefined {
  const text = header.trim();
  return headers.find(([, pattern]) => pattern.test(text))?.[0];
}

/** Check logical line starts once; whitespace-only Unicode line spans cannot be retried quadratically. */
export function hasRefusedDirective(body: string): boolean {
  const directive = /---|(?:click|style|classDef|linkStyle)\s/iy;
  let i = 0;
  while (i < body.length) {
    while (i < body.length && /\s/.test(body[i] ?? "")) i++;
    directive.lastIndex = i;
    if (directive.test(body)) return true;
    while (i < body.length && !/[\r\n\u2028\u2029]/.test(body[i] ?? "")) i++;
    i++;
  }
  return false;
}

/**
 * Bound the size, refuse configuration and styling directives (init blocks, YAML front matter,
 * click, style, classDef, linkStyle), strip comments, and find the family. Throws on any refusal.
 */
export function prepareDiagramSource(source: string): PreparedSource {
  if (source.length > MAX_SOURCE_CHARS) {
    throw new DiagramLimitError(
      `Diagram too large: more than ${String(MAX_SOURCE_CHARS)} characters`,
    );
  }
  const rawLines = source.split(/\r?\n/);
  if (rawLines.some((line) => line.length > MAX_LINE_CHARS)) {
    throw new DiagramLimitError(
      `Diagram line too long: more than ${String(MAX_LINE_CHARS)} characters`,
    );
  }
  const lines = rawLines
    .map((line, index) => (index === 0 ? line.replace(/^\uFEFF/, "") : line))
    .filter((line) => line.trim() !== "" && !line.trim().startsWith("%%"))
    .map((line) => line.trimEnd());
  const body = lines.join("\n");
  if (/%%\{/.test(source) || hasRefusedDirective(body)) {
    throw new DiagramUnsupportedError("Unsupported diagram: configuration or styling directive");
  }
  const first = lines[0];
  if (first === undefined) throw new DiagramInputError("Empty diagram");
  const kind = detectType(first);
  if (!kind) throw new DiagramUnsupportedError("Unsupported diagram: unknown header");
  return { kind, lines };
}

/** Engines accept raw text or an already prepared source; text is prepared again so a direct call is just as safe. */
export function linesOf(input: string | PreparedSource, kind: DiagramType): string[] {
  const prepared = typeof input === "string" ? prepareDiagramSource(input) : input;
  if (prepared.kind !== kind) throw new DiagramInputError(`Expected a ${kind} diagram`);
  return prepared.lines;
}
