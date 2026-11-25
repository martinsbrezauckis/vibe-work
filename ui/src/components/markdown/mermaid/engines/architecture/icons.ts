/**
 * Pure-SVG geometric icons for architecture diagrams (`cloud`, `database`, `disk`, `internet`,
 * `server`, plus a generic component badge for custom `pack:icon` names).
 */
import { circle, el, g, line, path, rect, type Attrs } from "../../svg";
import { color } from "../../theme";

const ellipse = (cx: number, cy: number, rx: number, ry: number, attrs: Attrs = {}): string =>
  el("ellipse", { cx, cy, rx, ry, ...attrs });

/** Draw a centered icon glyph inside a box of side `size` at `(cx, cy)`. */
export function drawArchIcon(
  iconName: string,
  cx: number,
  cy: number,
  size: number,
  stroke: string,
): string {
  const kind = iconName.trim().toLowerCase();
  const s = size / 24;
  const x0 = cx - size / 2;
  const y0 = cy - size / 2;
  const base = {
    fill: "none",
    stroke,
    "stroke-width": 1.5,
    "stroke-linecap": "round",
    "stroke-linejoin": "round",
  };

  if (kind === "cloud" || kind.endsWith(":cloud")) {
    const d = `M${String(x0 + 6 * s)},${String(y0 + 18 * s)} H${String(x0 + 18 * s)} A${String(4 * s)},${String(4 * s)} 0 0 0 ${String(x0 + 18.5 * s)},${String(y0 + 10.2 * s)} A${String(5.5 * s)},${String(5.5 * s)} 0 0 0 ${String(x0 + 7.8 * s)},${String(y0 + 8.6 * s)} A${String(4.2 * s)},${String(4.2 * s)} 0 0 0 ${String(x0 + 6 * s)},${String(y0 + 18 * s)} Z`;
    return path(d, { ...base, fill: stroke, "fill-opacity": 0.12 });
  }
  if (
    kind === "database" ||
    kind.includes("aurora") ||
    kind.includes("rds") ||
    kind.includes("db")
  ) {
    return g(
      {},
      [
        ellipse(cx, y0 + 6 * s, 7.5 * s, 2.8 * s, { ...base, fill: stroke, "fill-opacity": 0.14 }),
        path(
          `M${String(cx - 7.5 * s)},${String(y0 + 6 * s)} V${String(y0 + 18 * s)} A${String(7.5 * s)},${String(2.8 * s)} 0 0 0 ${String(cx + 7.5 * s)},${String(y0 + 18 * s)} V${String(y0 + 6 * s)}`,
          base,
        ),
        path(
          `M${String(cx - 7.5 * s)},${String(y0 + 12 * s)} A${String(7.5 * s)},${String(2.8 * s)} 0 0 0 ${String(cx + 7.5 * s)},${String(y0 + 12 * s)}`,
          base,
        ),
      ].join(""),
    );
  }
  if (
    kind === "disk" ||
    kind.includes("s3") ||
    kind.includes("glacier") ||
    kind.includes("storage")
  ) {
    return g(
      {},
      [
        rect(x0 + 4 * s, y0 + 4 * s, 16 * s, 16 * s, {
          rx: 2.5 * s,
          ...base,
          fill: stroke,
          "fill-opacity": 0.12,
        }),
        circle(cx, y0 + 11 * s, 4 * s, base),
        circle(cx, y0 + 11 * s, 1.3 * s, { fill: stroke }),
        line(x0 + 7 * s, y0 + 17.5 * s, x0 + 17 * s, y0 + 17.5 * s, base),
      ].join(""),
    );
  }
  if (kind === "internet" || kind.includes("globe") || kind.includes("gateway")) {
    return g(
      {},
      [
        circle(cx, cy, 8 * s, { ...base, fill: stroke, "fill-opacity": 0.12 }),
        ellipse(cx, cy, 3.8 * s, 8 * s, base),
        line(cx - 8 * s, cy, cx + 8 * s, cy, base),
      ].join(""),
    );
  }
  if (
    kind === "server" ||
    kind.includes("ec2") ||
    kind.includes("lambda") ||
    kind.includes("compute")
  ) {
    return g(
      {},
      [
        rect(x0 + 3.5 * s, y0 + 4 * s, 17 * s, 6.5 * s, {
          rx: 1.5 * s,
          ...base,
          fill: stroke,
          "fill-opacity": 0.12,
        }),
        rect(x0 + 3.5 * s, y0 + 13.5 * s, 17 * s, 6.5 * s, {
          rx: 1.5 * s,
          ...base,
          fill: stroke,
          "fill-opacity": 0.12,
        }),
        circle(x0 + 7 * s, y0 + 7.2 * s, 1 * s, { fill: stroke }),
        circle(x0 + 7 * s, y0 + 16.7 * s, 1 * s, { fill: stroke }),
        line(x0 + 11 * s, y0 + 7.2 * s, x0 + 17 * s, y0 + 7.2 * s, base),
        line(x0 + 11 * s, y0 + 16.7 * s, x0 + 17 * s, y0 + 16.7 * s, base),
      ].join(""),
    );
  }
  // Generic package/component icon when omitted or custom icon pack is referenced.
  return g(
    {},
    [
      rect(x0 + 4.5 * s, y0 + 4.5 * s, 15 * s, 15 * s, {
        rx: 2.5 * s,
        ...base,
        fill: stroke,
        "fill-opacity": 0.1,
      }),
      circle(cx, cy, 2.5 * s, { fill: color.textSec }),
    ].join(""),
  );
}
