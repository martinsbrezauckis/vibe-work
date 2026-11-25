/** Lazy, DOM-free Mermaid rendering. Called by MermaidBlock; only passiveMarkup's output reaches the page. */
import { prepareDiagramSource } from "./mermaid/detect";
import { engineFor } from "./mermaid/registry";

/**
 * Bound the size, refuse configuration and styling directives, find the family and draw it with its
 * engine. Any refusal or syntax problem throws; diagrams inherit the app's theme through tokens.
 */
export async function drawDiagram(source: string): Promise<string> {
  const prepared = prepareDiagramSource(source);
  const engine = await engineFor(prepared.kind);
  return engine.render(prepared);
}
