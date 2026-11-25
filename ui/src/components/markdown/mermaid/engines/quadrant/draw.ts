/**
 * Quadrant chart drawing: 2x2 tinted quadrant grid with optional top title, x/y axis captions,
 * quadrant headers (centered when empty, top-aligned when points exist), and plotted dots.
 */
import { clampLabel, fitWidth, textWidth, wrapText } from "../../measure";
import { circle, g, line, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { QuadrantModel } from "./parse";

const PLOT_W = 500;
const PLOT_H = 500;
const PAD = 20;
const POINT_R = 5;
const MAX_POINT_LABEL_W = 150;
const AXIS_BAND = 26;

function quadrantTintIndex(quadrantNumber: number): number {
  // Distinct subtle series tints for Q1 (top-right), Q2 (top-left), Q3 (bottom-left), Q4 (bottom-right).
  return quadrantNumber === 1 ? 4 : quadrantNumber === 2 ? 1 : quadrantNumber === 3 ? 3 : 0;
}

/** Draw a parsed quadrant chart as a complete passive SVG string. */
export function drawQuadrant(model: QuadrantModel): string {
  const hasPoints = model.points.length > 0;
  const hasXAxis = Boolean(model.xAxis.left || model.xAxis.right);
  const hasYAxis = Boolean(model.yAxis.bottom || model.yAxis.top);
  const titleText = model.title
    ? fitWidth(clampLabel(model.title), PLOT_W + PAD * 2, { size: FONT.title, bold: true })
    : "";
  const titleH = titleText ? 34 : 0;

  const pointStyle = { size: FONT.small };
  let extraLeft = 0;
  let extraRight = 0;
  let extraBottom = 0;
  const preparedPoints = model.points.map((point) => {
    const label = fitWidth(clampLabel(point.label), MAX_POINT_LABEL_W, pointStyle);
    const halfLabelW = textWidth(label, pointStyle) / 2;
    const rawX = point.x * PLOT_W;
    const rawY = (1 - point.y) * PLOT_H;
    extraLeft = Math.max(extraLeft, Math.max(POINT_R + 4, halfLabelW + 4) - rawX);
    extraRight = Math.max(extraRight, rawX + Math.max(POINT_R + 4, halfLabelW + 4) - PLOT_W);
    extraBottom = Math.max(extraBottom, rawY + POINT_R + 18 - PLOT_H);
    return { ...point, label };
  });

  const yAxisW = hasYAxis ? AXIS_BAND : 0;
  const xTopH = !hasPoints && hasXAxis ? AXIS_BAND : 0;
  const xBottomH = hasPoints && hasXAxis ? AXIS_BAND : 0;
  const plotLeft = PAD + yAxisW + Math.max(0, Math.ceil(extraLeft));
  const plotTop = PAD + titleH + xTopH;
  const plotBottom = plotTop + PLOT_H;
  const rightMargin = PAD + Math.max(0, Math.ceil(extraRight));
  const bottomMargin = PAD + Math.max(0, Math.ceil(extraBottom)) + xBottomH;
  const width = plotLeft + PLOT_W + rightMargin;
  const height = plotBottom + bottomMargin;

  const halfW = PLOT_W / 2;
  const halfH = PLOT_H / 2;
  const midX = plotLeft + halfW;
  const midY = plotTop + halfH;

  let body = "";
  if (titleText) {
    body += text(plotLeft + halfW, PAD + 10, titleText, {
      size: FONT.title,
      weight: "600",
      anchor: "middle",
    });
  }

  // Four quadrant cells (Q2 top-left, Q1 top-right, Q3 bottom-left, Q4 bottom-right).
  const cells: { num: number; x: number; y: number }[] = [
    { num: 1, x: midX, y: plotTop },
    { num: 2, x: plotLeft, y: plotTop },
    { num: 3, x: plotLeft, y: midY },
    { num: 4, x: midX, y: midY },
  ];
  for (const cell of cells) {
    body += rect(cell.x, cell.y, halfW, halfH, {
      fill: color.groupFill,
    });
    body += rect(cell.x, cell.y, halfW, halfH, {
      fill: series(quadrantTintIndex(cell.num)),
      "fill-opacity": 0.08,
    });
  }

  // Grid borders and center crosshairs.
  body += line(midX, plotTop, midX, plotBottom, { stroke: color.innerStroke, "stroke-width": 1 });
  body += line(plotLeft, midY, plotLeft + PLOT_W, midY, {
    stroke: color.innerStroke,
    "stroke-width": 1,
  });
  body += rect(plotLeft, plotTop, PLOT_W, PLOT_H, {
    fill: "none",
    stroke: color.nodeStroke,
    "stroke-width": 2,
  });

  // Quadrant titles.
  const qStyle = { size: FONT.body, bold: true };
  for (const cell of cells) {
    const rawLabel = model.quadrants[cell.num - 1] ?? "";
    if (!rawLabel) continue;
    const lines = wrapText(rawLabel, halfW - 20, qStyle, 2);
    const lineHeight = 1.25;
    const textY = hasPoints
      ? cell.y + 18
      : cell.y + halfH / 2 - ((lines.length - 1) * FONT.body * lineHeight) / 2;
    body += textLines(
      cell.x + halfW / 2,
      textY,
      lines,
      {
        size: FONT.body,
        weight: "600",
        anchor: "middle",
        fill: color.textSec,
      },
      lineHeight,
    );
  }

  // Axis captions.
  const axisStyle = { size: FONT.body, bold: true };
  if (hasXAxis) {
    const xLabelY = hasPoints ? plotBottom + Math.ceil(extraBottom) + 16 : plotTop - 12;
    if (model.xAxis.left) {
      body += text(
        plotLeft + halfW / 2,
        xLabelY,
        fitWidth(clampLabel(model.xAxis.left), halfW - 12, axisStyle),
        {
          size: FONT.body,
          weight: "600",
          anchor: "middle",
          fill: color.textSec,
        },
      );
    }
    if (model.xAxis.right) {
      body += text(
        midX + halfW / 2,
        xLabelY,
        fitWidth(clampLabel(model.xAxis.right), halfW - 12, axisStyle),
        {
          size: FONT.body,
          weight: "600",
          anchor: "middle",
          fill: color.textSec,
        },
      );
    }
  }
  if (hasYAxis) {
    const yAxisX = plotLeft - Math.max(14, Math.ceil(extraLeft) + 14);
    if (model.yAxis.bottom) {
      const cy = midY + halfH / 2;
      body += g(
        {
          transform: `translate(${String(Math.round(yAxisX))},${String(Math.round(cy))}) rotate(-90)`,
        },
        text(0, 0, fitWidth(clampLabel(model.yAxis.bottom), halfH - 12, axisStyle), {
          size: FONT.body,
          weight: "600",
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
    if (model.yAxis.top) {
      const cy = plotTop + halfH / 2;
      body += g(
        {
          transform: `translate(${String(Math.round(yAxisX))},${String(Math.round(cy))}) rotate(-90)`,
        },
        text(0, 0, fitWidth(clampLabel(model.yAxis.top), halfH - 12, axisStyle), {
          size: FONT.body,
          weight: "600",
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
  }

  // Plotted points and labels.
  const pointsMarkup = preparedPoints
    .map((point) => {
      const cx = plotLeft + point.x * PLOT_W;
      const cy = plotTop + (1 - point.y) * PLOT_H;
      return (
        circle(cx, cy, POINT_R, {
          fill: color.accent,
          stroke: color.canvas,
          "stroke-width": 1.5,
        }) +
        text(cx, cy + POINT_R + 10, point.label, {
          size: FONT.small,
          weight: "500",
          anchor: "middle",
          fill: color.text,
        })
      );
    })
    .join("");
  if (pointsMarkup) body += g({}, pointsMarkup);

  return svgDoc(width, height, body);
}
