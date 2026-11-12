/** Lazy KaTeX-to-MathML conversion. No HTML layout/fonts, resource commands or shared macros; called by MathFormula. */
import katex from "katex";

/** Trust stays off. Bound macro expansion and dimensions independently of the source length. */
export function drawMath(source: string, display: boolean): string {
  if (source.length > 8000) throw new Error("Formula too large");
  return katex.renderToString(source, {
    displayMode: display,
    output: "mathml",
    trust: false,
    strict: "error",
    throwOnError: true,
    maxExpand: 500,
    maxSize: 20,
    macros: {},
  });
}
