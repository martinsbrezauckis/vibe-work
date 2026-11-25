/**
 * Radar chart drawing: polar graticule (circle or polygon), radial spokes, scale tick labels,
 * semi-transparent multi-series polygons with vertex markers, dynamic axis margins, and legend.
 */
import { clampLabel, fitWidth, textWidth } from "../../measure";
import { circle, g, line, polygon, polyline, rect, svgDoc, text } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { RadarModel } from "./parse";

const RADIUS = 165;
const PAD = 24;
const LABEL_OFFSET = 14;
const MAX_AXIS_LABEL_W = 140;
const LEGEND_SWATCH = 10;
const LEGEND_GAP = 18;
const LEGEND_ROW_H = 22;

function formatTick(value: number): string {
  const rounded = Math.round(value * 100) / 100;
  return String(rounded);
}

interface PlacedLegendItem {
  x: number;
  y: number;
  label: string;
  colorIndex: number;
}

function layoutLegend(
  labels: readonly string[],
  maxRowWidth: number,
  topY: number,
  centerX: number,
): { items: PlacedLegendItem[]; height: number } {
  if (labels.length === 0) return { items: [], height: 0 };
  const style = { size: FONT.body };
  const rows: { items: { label: string; width: number; colorIndex: number }[]; width: number }[] = [
    { items: [], width: 0 },
  ];
  labels.forEach((raw, index) => {
    const label = fitWidth(clampLabel(raw), 180, style);
    const w = LEGEND_SWATCH + 6 + textWidth(label, style);
    const current = rows[rows.length - 1];
    if (!current) return;
    const needed = current.items.length === 0 ? w : current.width + LEGEND_GAP + w;
    if (current.items.length > 0 && needed > maxRowWidth) {
      rows.push({ items: [{ label, width: w, colorIndex: index }], width: w });
    } else {
      current.items.push({ label, width: w, colorIndex: index });
      current.width = needed;
    }
  });
  const items: PlacedLegendItem[] = [];
  rows.forEach((row, rowIndex) => {
    let cursor = centerX - row.width / 2;
    const y = topY + rowIndex * LEGEND_ROW_H + LEGEND_ROW_H / 2;
    for (const item of row.items) {
      items.push({ x: cursor, y, label: item.label, colorIndex: item.colorIndex });
      cursor += item.width + LEGEND_GAP;
    }
  });
  return { items, height: rows.length * LEGEND_ROW_H };
}

/** Draw a parsed radar chart as a complete passive SVG string. */
export function drawRadar(model: RadarModel): string {
  const axisCount = model.axes.length;
  const angles = model.axes.map((_, i) => -Math.PI / 2 + (i * Math.PI * 2) / axisCount);
  const axisStyle = { size: FONT.small, bold: true };
  const axisLabels = model.axes.map((ax, i) => {
    const angle = angles[i] ?? 0;
    const cos = Math.cos(angle);
    const sin = Math.sin(angle);
    const label = fitWidth(clampLabel(ax.label), MAX_AXIS_LABEL_W, axisStyle);
    const w = textWidth(label, axisStyle);
    const anchor: "start" | "middle" | "end" =
      Math.abs(cos) < 0.2 ? "middle" : cos > 0 ? "start" : "end";
    const lx = cos * (RADIUS + LABEL_OFFSET);
    const ly = sin * (RADIUS + LABEL_OFFSET);
    const leftEdge = anchor === "start" ? lx : anchor === "end" ? lx - w : lx - w / 2;
    const rightEdge = anchor === "start" ? lx + w : anchor === "end" ? lx : lx + w / 2;
    return { label, anchor, lx, ly, leftEdge, rightEdge };
  });

  let minX = -RADIUS;
  let maxX = RADIUS;
  let minY = -RADIUS;
  let maxY = RADIUS;
  for (const lbl of axisLabels) {
    minX = Math.min(minX, lbl.leftEdge);
    maxX = Math.max(maxX, lbl.rightEdge);
    minY = Math.min(minY, lbl.ly - FONT.small);
    maxY = Math.max(maxY, lbl.ly + FONT.small);
  }

  const titleText = model.title
    ? fitWidth(clampLabel(model.title), Math.max(360, maxX - minX), {
        size: FONT.title,
        bold: true,
      })
    : "";
  const titleH = titleText ? 34 : 0;
  const radarW = maxX - minX + PAD * 2;
  const titleW = titleText ? textWidth(titleText, { size: FONT.title, bold: true }) + PAD * 2 : 0;
  const width = Math.max(radarW, titleW, 380);
  const extraCenterX = (width - (maxX - minX + PAD * 2)) / 2;
  const cx = PAD + extraCenterX - minX;
  const cy = PAD + titleH - minY;
  const radarBottom = cy + maxY;

  const legend = model.showLegend
    ? layoutLegend(
        model.curves.map((c) => c.label),
        Math.max(300, width - PAD * 2),
        radarBottom + 14,
        width / 2,
      )
    : { items: [], height: 0 };
  const height = radarBottom + (legend.height > 0 ? 14 + legend.height : 0) + PAD;

  let body = "";
  if (titleText) {
    body += text(width / 2, PAD + 10, titleText, {
      size: FONT.title,
      weight: "600",
      anchor: "middle",
    });
  }

  // Concentric graticule rings and top-spoke tick labels.
  const graticuleMarkup: string[] = [];
  for (let t = 1; t <= model.ticks; t++) {
    const frac = t / model.ticks;
    const r = frac * RADIUS;
    if (model.graticule === "polygon" && axisCount >= 3) {
      const pts: [number, number][] = angles.map((a) => [
        cx + r * Math.cos(a),
        cy + r * Math.sin(a),
      ]);
      graticuleMarkup.push(
        polygon(pts, {
          fill: "none",
          stroke: color.innerStroke,
          "stroke-width": 1,
          "stroke-opacity": 0.65,
        }),
      );
    } else {
      graticuleMarkup.push(
        circle(cx, cy, r, {
          fill: "none",
          stroke: color.innerStroke,
          "stroke-width": 1,
          "stroke-opacity": 0.65,
        }),
      );
    }
    const tickVal = model.min + frac * (model.max - model.min);
    graticuleMarkup.push(
      text(cx + 4, cy - r - 2, formatTick(tickVal), {
        size: FONT.small,
        anchor: "start",
        fill: color.textFaint,
      }),
    );
  }
  body += g({}, graticuleMarkup.join(""));

  // Radial spokes and axis captions.
  const spokesAndLabels = angles
    .map((a, i) => {
      const lbl = axisLabels[i];
      if (!lbl) return "";
      const x2 = cx + RADIUS * Math.cos(a);
      const y2 = cy + RADIUS * Math.sin(a);
      return (
        line(cx, cy, x2, y2, { stroke: color.line, "stroke-width": 1 }) +
        text(cx + lbl.lx, cy + lbl.ly, lbl.label, {
          size: FONT.small,
          weight: "600",
          anchor: lbl.anchor,
          fill: color.textSec,
        })
      );
    })
    .join("");
  body += g({}, spokesAndLabels);

  // Data curves and vertex dots.
  const span = model.max - model.min;
  const curvesMarkup = model.curves
    .map((curve, ci) => {
      const stroke = series(ci);
      const pts: [number, number][] = angles.map((a, i) => {
        const val = curve.values[i] ?? model.min;
        const frac = Math.max(0, Math.min(1, (val - model.min) / span));
        const r = frac * RADIUS;
        return [cx + r * Math.cos(a), cy + r * Math.sin(a)];
      });
      const shape =
        axisCount >= 3
          ? polygon(pts, {
              fill: stroke,
              "fill-opacity": 0.2,
              stroke,
              "stroke-width": 2,
              "stroke-linejoin": "round",
            })
          : polyline(pts, { stroke, "stroke-width": 2 });
      const dots = pts
        .map(([px, py]) =>
          circle(px, py, 3.5, { fill: stroke, stroke: color.canvas, "stroke-width": 1 }),
        )
        .join("");
      return shape + dots;
    })
    .join("");
  body += g({}, curvesMarkup);

  // Legend.
  if (legend.items.length > 0) {
    const legendMarkup = legend.items
      .map(
        (item) =>
          rect(item.x, item.y - LEGEND_SWATCH / 2, LEGEND_SWATCH, LEGEND_SWATCH, {
            rx: 2,
            fill: series(item.colorIndex),
          }) +
          text(item.x + LEGEND_SWATCH + 6, item.y, item.label, {
            size: FONT.body,
            fill: color.text,
          }),
      )
      .join("");
    body += g({}, legendMarkup);
  }

  return svgDoc(width, height, body);
}
