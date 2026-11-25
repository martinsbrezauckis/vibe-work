/** Pie chart drawing: wedges clockwise from twelve o'clock, largest first, with a key that carries the numbers. */
import { clampLabel, fitWidth, textWidth } from "../../measure";
import { circle, g, n, path, rect, svgDoc, text } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { PieModel } from "./parse";

const RADIUS = 110;
const PAD = 20;
const ROW = 24;
const KEY_TEXT_MAX = 260;

function percent(value: number, total: number): string {
  const p = (value / total) * 100;
  return `${p >= 10 || Number.isInteger(p) ? p.toFixed(0) : p.toFixed(1)}%`;
}

/** Draw a parsed pie chart as a complete SVG string. */
export function drawPie(model: PieModel): string {
  const slices = model.slices
    .map((slice, index) => ({ ...slice, index }))
    .sort((a, b) => b.value - a.value || a.index - b.index);
  const style = { size: FONT.body };
  const entries = slices.map((slice) => {
    const label = fitWidth(clampLabel(slice.label), KEY_TEXT_MAX, style);
    const figures = model.showData
      ? `${percent(slice.value, model.total)}  [${String(slice.value)}]`
      : percent(slice.value, model.total);
    return { slice, label, figures };
  });
  const keyLabelWidth = Math.max(0, ...entries.map((e) => textWidth(e.label, style)));
  const keyFigureWidth = Math.max(0, ...entries.map((e) => textWidth(e.figures, style)));
  const keyWidth = 18 + keyLabelWidth + 16 + keyFigureWidth;
  const titleText = model.title
    ? fitWidth(clampLabel(model.title), 520, { size: FONT.title, bold: true })
    : "";
  const titleHeight = titleText ? 32 : 0;
  const keyHeight = entries.length * ROW;
  const chartHeight = Math.max(RADIUS * 2, keyHeight);
  const width = Math.max(
    PAD * 3 + RADIUS * 2 + keyWidth,
    titleText ? textWidth(titleText, { size: FONT.title, bold: true }) + PAD * 2 : 0,
  );
  const height = PAD * 2 + titleHeight + chartHeight;
  const cx = PAD + RADIUS;
  const cy = PAD + titleHeight + chartHeight / 2;
  let body = "";
  if (titleText) {
    body += text(width / 2, PAD + 8, titleText, {
      size: FONT.title,
      weight: "600",
      anchor: "middle",
    });
  }
  let angle = -Math.PI / 2;
  const wedges: string[] = [];
  for (const { slice } of entries) {
    const sweep = (slice.value / model.total) * Math.PI * 2;
    const fill = series(slice.index);
    if (slice.value / model.total >= 0.9999) {
      wedges.push(circle(cx, cy, RADIUS, { fill, stroke: color.canvas, "stroke-width": 2 }));
    } else if (slice.value > 0) {
      const x1 = cx + RADIUS * Math.cos(angle);
      const y1 = cy + RADIUS * Math.sin(angle);
      const x2 = cx + RADIUS * Math.cos(angle + sweep);
      const y2 = cy + RADIUS * Math.sin(angle + sweep);
      const large = sweep > Math.PI ? 1 : 0;
      wedges.push(
        path(
          `M${n(cx)},${n(cy)} L${n(x1)},${n(y1)} A${n(RADIUS)},${n(RADIUS)} 0 ${String(large)} 1 ${n(x2)},${n(y2)} Z`,
          {
            fill,
            stroke: color.canvas,
            "stroke-width": 2,
            "stroke-linejoin": "round",
          },
        ),
      );
    }
    angle += sweep;
  }
  body += g({}, wedges.join(""));
  const keyX = PAD * 2 + RADIUS * 2;
  const keyTop = PAD + titleHeight + (chartHeight - keyHeight) / 2;
  entries.forEach(({ slice, label, figures }, row) => {
    const y = keyTop + row * ROW + ROW / 2;
    body += rect(keyX, y - 6, 12, 12, { rx: 2, fill: series(slice.index) });
    body += text(keyX + 18, y, label, { size: FONT.body });
    body += text(keyX + 18 + keyLabelWidth + 16 + keyFigureWidth, y, figures, {
      size: FONT.body,
      anchor: "end",
      fill: color.textSec,
    });
  });
  return svgDoc(width, height, body);
}
