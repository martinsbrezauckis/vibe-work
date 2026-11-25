/**
 * Pure string builders for SVG. Every attribute value is a number, an app colour token or an
 * allowlisted keyword; user text only ever appears as escaped element content. Tag and attribute
 * names are checked against the same vocabulary passiveMarkup keeps, so a bug here fails in tests
 * instead of being silently dropped on the page.
 */
import { checkSide } from "./limits";
import { color } from "./theme";

const TAGS = new Set(
  "svg g path rect circle ellipse line polyline polygon text tspan defs marker title desc".split(
    " ",
  ),
);
const ATTRS = new Set(
  "id viewBox width height x y x1 x2 y1 y2 dx dy cx cy r rx ry d points transform fill stroke stroke-width stroke-dasharray stroke-linecap stroke-linejoin opacity fill-opacity stroke-opacity font-size font-weight font-style text-anchor dominant-baseline markerWidth markerHeight refX refY orient markerUnits preserveAspectRatio marker-start marker-end".split(
    " ",
  ),
);

/** Attribute map: numbers are formatted, undefined values are skipped. */
export type Attrs = Record<string, string | number | undefined>;

/** XML-escape text and drop characters XML 1.0 cannot carry. */
export function esc(text: string): string {
  let out = "";
  for (const character of text) {
    const code = character.codePointAt(0) ?? 0;
    if (code < 32 || (code >= 0xd800 && code <= 0xdfff) || code === 0xfffe || code === 0xffff) {
      if (code === 9 || code === 10) out += " ";
      continue;
    }
    out +=
      character === "&"
        ? "&amp;"
        : character === "<"
          ? "&lt;"
          : character === ">"
            ? "&gt;"
            : character === '"'
              ? "&quot;"
              : character === "'"
                ? "&apos;"
                : character;
  }
  return out;
}

/** Format a coordinate: finite, at most two decimals. */
export function n(value: number): string {
  if (!Number.isFinite(value)) throw new Error("Non-finite coordinate");
  return String(Math.round(value * 100) / 100);
}

function attrString(attrs: Attrs): string {
  let out = "";
  for (const [name, raw] of Object.entries(attrs)) {
    if (raw === undefined) continue;
    if (!ATTRS.has(name)) throw new Error(`Attribute not allowed in drawings: ${name}`);
    const value = typeof raw === "number" ? n(raw) : raw;
    if (/[<>"&]/.test(value)) throw new Error(`Unsafe attribute value for ${name}`);
    out += ` ${name}="${value}"`;
  }
  return out;
}

/** Build one element; `inner` is already-built markup (never user text). */
export function el(tag: string, attrs: Attrs = {}, inner = ""): string {
  if (!TAGS.has(tag)) throw new Error(`Tag not allowed in drawings: ${tag}`);
  return inner === "" && tag !== "text"
    ? `<${tag}${attrString(attrs)}/>`
    : `<${tag}${attrString(attrs)}>${inner}</${tag}>`;
}

/** A group element. */
export const g = (attrs: Attrs, inner: string): string => el("g", attrs, inner);

/** A rectangle; `attrs` may add rounding, fill and stroke. */
export function rect(
  x: number,
  y: number,
  width: number,
  height: number,
  attrs: Attrs = {},
): string {
  return el("rect", { x, y, width, height, ...attrs });
}
/** A straight line. */
export function line(x1: number, y1: number, x2: number, y2: number, attrs: Attrs = {}): string {
  return el("line", { x1, y1, x2, y2, ...attrs });
}
/** A circle. */
export function circle(cx: number, cy: number, r: number, attrs: Attrs = {}): string {
  return el("circle", { cx, cy, r, ...attrs });
}
/** A path from its `d` string. */
export function path(d: string, attrs: Attrs = {}): string {
  return el("path", { d, ...attrs });
}
/** A closed polygon from points. */
export function polygon(points: readonly [number, number][], attrs: Attrs = {}): string {
  return el("polygon", { points: points.map(([x, y]) => `${n(x)},${n(y)}`).join(" "), ...attrs });
}
/** An open polyline from points, unfilled by default. */
export function polyline(points: readonly [number, number][], attrs: Attrs = {}): string {
  return el("polyline", {
    points: points.map(([x, y]) => `${n(x)},${n(y)}`).join(" "),
    fill: "none",
    ...attrs,
  });
}

/** Size, weight, anchor, fill and italics of a text run. */
export interface TextOptions {
  size?: number;
  weight?: "400" | "500" | "600" | "700";
  anchor?: "start" | "middle" | "end";
  fill?: string;
  italic?: boolean;
}

/** One line of text centred vertically on `y`. */
export function text(x: number, y: number, content: string, options: TextOptions = {}): string {
  return el(
    "text",
    {
      x,
      y,
      dy: "0.35em",
      "text-anchor": options.anchor ?? "start",
      "font-size": options.size ?? 13,
      "font-weight": options.weight ?? "400",
      "font-style": options.italic ? "italic" : undefined,
      fill: options.fill ?? color.text,
    },
    esc(content),
  );
}

/** Several lines, each centred on its own row; `y` is the centre of the first row. */
export function textLines(
  x: number,
  y: number,
  lines: readonly string[],
  options: TextOptions = {},
  lineHeight = 1.3,
): string {
  const step = (options.size ?? 13) * lineHeight;
  return lines.map((content, index) => text(x, y + index * step, content, options)).join("");
}

/** A marker body in a 10 by 10 box and the x position of its tip. */
export interface MarkerDef {
  id: string;
  /** Marker body in a 10x10 box with the tip at x=10 (or the centre for open shapes). */
  body: string;
  refX: number;
}

const STROKE = { fill: "none", stroke: color.arrow, "stroke-width": 1.2 } as const;

/** Standard end markers shared by the connector-based engines. */
export const markers = {
  "arrow-filled": {
    id: "arrow-filled",
    refX: 10,
    body: el("polygon", { points: "0,1 10,5 0,9", fill: color.arrow }),
  },
  "arrow-open": {
    id: "arrow-open",
    refX: 10,
    body: el("polyline", { points: "1,1 9,5 1,9", ...STROKE }),
  },
  "arrow-cross": {
    id: "arrow-cross",
    refX: 8,
    body: el("path", { d: "M2,2 L8,8 M8,2 L2,8", ...STROKE }),
  },
  "triangle-hollow": {
    id: "triangle-hollow",
    refX: 10,
    body: el("polygon", {
      points: "1,1 10,5 1,9",
      fill: color.canvas,
      stroke: color.arrow,
      "stroke-width": 1.2,
    }),
  },
  "diamond-filled": {
    id: "diamond-filled",
    refX: 10,
    body: el("polygon", { points: "0,5 5,1 10,5 5,9", fill: color.arrow }),
  },
  "diamond-hollow": {
    id: "diamond-hollow",
    refX: 10,
    body: el("polygon", {
      points: "0,5 5,1 10,5 5,9",
      fill: color.canvas,
      stroke: color.arrow,
      "stroke-width": 1.2,
    }),
  },
} satisfies Record<string, MarkerDef>;

/** Name of a standard end marker. */
export type MarkerName = keyof typeof markers;

/** `<defs>` holding the named markers (drawn in a 10x10 user-space box). */
export function markerDefs(names: readonly MarkerName[]): string {
  const items = names.map((name) => {
    const m: MarkerDef = markers[name];
    return el(
      "marker",
      {
        id: m.id,
        markerWidth: 10,
        markerHeight: 10,
        refX: m.refX,
        refY: 5,
        orient: "auto-start-reverse",
        markerUnits: "userSpaceOnUse",
        viewBox: "0 0 10 10",
      },
      m.body,
    );
  });
  return el("defs", {}, items.join(""));
}

/** Reference an end marker defined by markerDefs. */
export const markerRef = (name: MarkerName): string => `url(#${name})`;

/** The complete document. Width and height are capped; viewBox and size match. */
export function svgDoc(width: number, height: number, body: string, defs = ""): string {
  const w = checkSide(Math.ceil(width));
  const h = checkSide(Math.ceil(height));
  return el("svg", { viewBox: `0 0 ${String(w)} ${String(h)}`, width: w, height: h }, defs + body);
}
