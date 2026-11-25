/**
 * Colour and type tokens for drawings. Only references to the app's own custom properties are ever
 * written into a drawing: the `--_*` names that `.md-diagram` defines in styles/markdown-rich.css
 * and the semantic and category colours of styles/tokens.css, which both themes define. No literal
 * colour exists in any drawing, so a theme switch re-colours drawings live without redrawing them.
 */
export const color = {
  text: "var(--_text)",
  textSec: "var(--_text-sec)",
  textMuted: "var(--_text-muted)",
  textFaint: "var(--_text-faint)",
  line: "var(--_line)",
  arrow: "var(--_arrow)",
  nodeFill: "var(--_node-fill)",
  nodeStroke: "var(--_node-stroke)",
  groupFill: "var(--_group-fill)",
  groupHdr: "var(--_group-hdr)",
  innerStroke: "var(--_inner-stroke)",
  keyBadge: "var(--_key-badge)",
  /** The colour behind a drawing (the code-block panel): label pills and hollow markers are filled with it. */
  canvas: "var(--panel)",
  accent: "var(--orange)",
  ok: "var(--green)",
  warn: "var(--orange-3)",
  crit: "var(--red)",
  info: "var(--blue)",
} as const;

/** The eight distinguishable category colours, cycled for slices, bars and branches. */
export const SERIES_COUNT = 8;
const SERIES = [
  "var(--agent-frame-0)",
  "var(--agent-frame-1)",
  "var(--agent-frame-2)",
  "var(--agent-frame-3)",
  "var(--agent-frame-4)",
  "var(--agent-frame-5)",
  "var(--agent-frame-6)",
  "var(--agent-frame-7)",
] as const;
/** The category colour token for an index, cycling through the eight series colours. */
export function series(index: number): string {
  return SERIES[((index % SERIES_COUNT) + SERIES_COUNT) % SERIES_COUNT] ?? SERIES[0];
}

/** Font sizes in user units. */
export const FONT = { small: 11, body: 13, title: 16 } as const;
