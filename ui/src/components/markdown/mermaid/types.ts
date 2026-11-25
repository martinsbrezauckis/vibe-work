/** Shared contracts of the Mermaid drawing framework. Everything here is DOM-free and pure. */

/**
 * Every diagram family Mermaid documents and this app knows a header for. A family without an
 * engine in the registry is refused as unsupported and the chat keeps the source as code.
 */
export type DiagramType =
  | "flowchart"
  | "sequence"
  | "class"
  | "state"
  | "er"
  | "pie"
  | "gantt"
  | "quadrant"
  | "xychart"
  | "sankey"
  | "radar"
  | "journey"
  | "timeline"
  | "gitgraph"
  | "kanban"
  | "c4"
  | "requirement"
  | "architecture"
  | "block"
  | "packet"
  | "mindmap"
  | "treemap"
  | "zenuml"
  | "swimlane"
  | "usecase"
  | "eventmodeling"
  | "ishikawa"
  | "treeview"
  | "agentflow"
  | "venn"
  | "wardley"
  | "cynefin";

/** Registry slot that owns a family; one source file per group keeps additions independent. */
export type DiagramGroup =
  "core" | "charts" | "timelines" | "architecture" | "hierarchy" | "specialty";

/** Comment-stripped, directive-checked source handed to an engine. */
export interface PreparedSource {
  kind: DiagramType;
  /** Header line first, then the body; blank lines and %% comment lines are gone, indentation is kept. */
  lines: string[];
}

/**
 * One diagram family. `render` parses (throwing a DiagramError on any problem) and draws.
 * The returned string must be a complete `<svg>` built only with ../svg.ts helpers.
 */
export interface DiagramEngine {
  render(prepared: PreparedSource): string | Promise<string>;
}

/** Group modules export their engines as lazy loaders so a chat only pays for the family it shows. */
export type EngineLoader = () => Promise<DiagramEngine>;
/** Engines of one group by family; absent families are unsupported. */
export type GroupEngines = Partial<Record<DiagramType, EngineLoader>>;

/** The source is not a valid diagram (syntax error, truncated, empty, unknown reference). */
export class DiagramInputError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DiagramInputError";
  }
}

/** A size, nesting or step bound was exceeded; the message names the bound. */
export class DiagramLimitError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DiagramLimitError";
  }
}

/** The header is recognised or unknown but no engine draws it yet, or the source holds a refused directive. */
export class DiagramUnsupportedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DiagramUnsupportedError";
  }
}
