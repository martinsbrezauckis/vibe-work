/**
 * XY chart drawing: vertical and horizontal bar/line plots with nice numeric ticks, dashed gridlines,
 * categorical or numeric x-axes, grouped bars from zero baseline, per-point line labels, and legend.
 */
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, textWidth } from "../../measure";
import { circle, g, line, polyline, rect, svgDoc, text } from "../../svg";
import { FONT, color, series as seriesColor } from "../../theme";
import type { XyChartModel, XySeries } from "./parse";

const PAD = 20;
const TICK_LEN = 5;
const LEGEND_ROW_H = 22;
const LEGEND_GAP = 18;

function formatNum(value: number): string {
  const rounded = Math.round(value * 1000) / 1000;
  return Object.is(rounded, -0) ? "0" : String(rounded);
}

function niceStep(range: number, maxTicks: number): number {
  const raw = range / Math.max(1, maxTicks - 1);
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  const mult = norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 5 ? 5 : 10;
  return mult * mag;
}

function computeNumericTicks(
  minVal: number,
  maxVal: number,
  autoRange: boolean,
  includeZero: boolean,
  budget: StepBudget,
): { min: number; max: number; ticks: number[] } {
  let lo = minVal;
  let hi = maxVal;
  if (autoRange && includeZero) {
    lo = Math.min(0, lo);
    hi = Math.max(0, hi);
  }
  if (hi <= lo) {
    lo -= 1;
    hi += 1;
  }
  const step = niceStep(hi - lo, 6);
  if (autoRange) {
    lo = Math.floor(lo / step) * step;
    hi = Math.ceil(hi / step) * step;
    if (hi <= lo) hi = lo + step;
  }
  const ticks: number[] = [];
  const start = Math.ceil((lo - 1e-9 * step) / step) * step;
  for (let v = start; v <= hi + 1e-9 * step; v += step) {
    budget.tick();
    const cleaned = Math.round(v * 1e9) / 1e9;
    if (cleaned >= lo - 1e-9 && cleaned <= hi + 1e-9) ticks.push(cleaned);
    if (ticks.length > 20) break;
  }
  if (ticks.length === 0) ticks.push(lo, hi);
  return { min: lo, max: hi, ticks };
}

function resolveCategories(
  model: XyChartModel,
  budget: StepBudget,
): {
  categories: string[];
  numericX?: { min: number; max: number; ticks: number[] };
} {
  if (model.xAxis.kind === "categorical") {
    return { categories: model.xAxis.categories };
  }
  if (model.xAxis.kind === "numeric") {
    const info = computeNumericTicks(model.xAxis.min, model.xAxis.max, false, false, budget);
    return { categories: [], numericX: info };
  }
  const maxLen = Math.max(...model.series.map((s) => s.points.length));
  return { categories: Array.from({ length: maxLen }, (_, i) => String(i + 1)) };
}

interface PlacedLegend {
  x: number;
  y: number;
  kind: "bar" | "line";
  label: string;
  colorIndex: number;
}

function layoutLegend(
  allSeries: readonly XySeries[],
  maxWidth: number,
  topY: number,
  centerX: number,
): { items: PlacedLegend[]; height: number } {
  const named = allSeries
    .map((s, index) => ({ s, index }))
    .filter((entry): entry is { s: XySeries & { name: string }; index: number } =>
      Boolean(entry.s.name),
    );
  if (named.length === 0) return { items: [], height: 0 };
  const style = { size: FONT.body };
  const rows: {
    items: { kind: "bar" | "line"; label: string; width: number; colorIndex: number }[];
    width: number;
  }[] = [{ items: [], width: 0 }];
  for (const entry of named) {
    const label = fitWidth(clampLabel(entry.s.name), 180, style);
    const w = 18 + textWidth(label, style);
    const cur = rows[rows.length - 1];
    if (!cur) continue;
    const needed = cur.items.length === 0 ? w : cur.width + LEGEND_GAP + w;
    if (cur.items.length > 0 && needed > maxWidth) {
      rows.push({
        items: [{ kind: entry.s.kind, label, width: w, colorIndex: entry.index }],
        width: w,
      });
    } else {
      cur.items.push({ kind: entry.s.kind, label, width: w, colorIndex: entry.index });
      cur.width = needed;
    }
  }
  const items: PlacedLegend[] = [];
  rows.forEach((row, r) => {
    let cursor = centerX - row.width / 2;
    const y = topY + r * LEGEND_ROW_H + LEGEND_ROW_H / 2;
    for (const item of row.items) {
      items.push({
        x: cursor,
        y,
        kind: item.kind,
        label: item.label,
        colorIndex: item.colorIndex,
      });
      cursor += item.width + LEGEND_GAP;
    }
  });
  return { items, height: rows.length * LEGEND_ROW_H };
}

function drawLegendItems(items: readonly PlacedLegend[]): string {
  if (items.length === 0) return "";
  return g(
    {},
    items
      .map((item) => {
        const fill = seriesColor(item.colorIndex);
        const swatch =
          item.kind === "bar"
            ? rect(item.x, item.y - 5, 12, 10, { rx: 2, fill })
            : line(item.x, item.y, item.x + 12, item.y, { stroke: fill, "stroke-width": 2 }) +
              circle(item.x + 6, item.y, 3, { fill });
        return (
          swatch + text(item.x + 18, item.y, item.label, { size: FONT.body, fill: color.text })
        );
      })
      .join(""),
  );
}

/** Draw a parsed XY chart (vertical or horizontal) as a complete passive SVG string. */
export function drawXyChart(model: XyChartModel): string {
  const budget = new StepBudget();
  const hasBars = model.series.some((s) => s.kind === "bar");
  const allVals = model.series.flatMap((s) => s.points.map((p) => p.value));
  const yInfo =
    model.yAxis.kind === "numeric"
      ? computeNumericTicks(model.yAxis.min, model.yAxis.max, false, hasBars, budget)
      : computeNumericTicks(Math.min(...allVals), Math.max(...allVals), true, hasBars, budget);
  const xInfo = resolveCategories(model, budget);
  const slotCount = xInfo.numericX
    ? Math.max(1, ...model.series.map((s) => s.points.length))
    : xInfo.categories.length;

  const titleStyle = { size: FONT.title, bold: true };
  const tickStyle = { size: FONT.small };
  const axisTitleStyle = { size: FONT.body, bold: true };
  const hasLinePointLabels = model.series.some(
    (s) => s.kind === "line" && s.points.some((p) => Boolean(p.label)),
  );

  if (model.orientation === "vertical") {
    return drawVertical(model, yInfo, xInfo, slotCount, {
      titleStyle,
      tickStyle,
      axisTitleStyle,
      hasLinePointLabels,
    });
  }
  return drawHorizontal(model, yInfo, xInfo, slotCount, {
    titleStyle,
    tickStyle,
    axisTitleStyle,
    hasLinePointLabels,
  });
}

interface DrawStyles {
  titleStyle: { size: number; bold: boolean };
  tickStyle: { size: number };
  axisTitleStyle: { size: number; bold: boolean };
  hasLinePointLabels: boolean;
}

function drawVertical(
  model: XyChartModel,
  yInfo: { min: number; max: number; ticks: number[] },
  xInfo: { categories: string[]; numericX?: { min: number; max: number; ticks: number[] } },
  slotCount: number,
  styles: DrawStyles,
): string {
  const plotW = Math.max(500, Math.min(2400, slotCount * 22));
  const plotH = 340;
  const yTickLabels = yInfo.ticks.map(formatNum);
  const maxYTickW = Math.max(24, ...yTickLabels.map((l) => textWidth(l, styles.tickStyle)));
  const yTitleSpace = model.yAxis.title ? 24 : 0;
  const leftMargin = PAD + yTitleSpace + maxYTickW + 12;

  const xLabels = xInfo.numericX
    ? xInfo.numericX.ticks.map(formatNum)
    : xInfo.categories.map((c) => fitWidth(clampLabel(c), 120, styles.tickStyle));
  const slotW = plotW / Math.max(1, slotCount);
  const rotateX =
    !xInfo.numericX && xLabels.some((l) => textWidth(l, styles.tickStyle) > slotW - 6);
  const maxXLabelW = Math.max(0, ...xLabels.map((l) => textWidth(l, styles.tickStyle)));
  const xLabelBand = rotateX ? Math.min(90, Math.ceil(maxXLabelW * 0.65) + 14) : 24;
  const xTitleSpace = model.xAxis.title ? 24 : 0;

  const titleText = model.title
    ? fitWidth(clampLabel(model.title), leftMargin + plotW, styles.titleStyle)
    : "";
  const titleH = titleText ? 34 : 0;
  const topLabelRoom = styles.hasLinePointLabels ? 18 : 0;
  const plotTop = PAD + titleH + topLabelRoom;
  const plotLeft = leftMargin;
  const plotRight = plotLeft + plotW;
  const plotBottom = plotTop + plotH;
  const width = plotRight + PAD + (styles.hasLinePointLabels ? 24 : 0);
  const legendTop = plotBottom + xLabelBand + xTitleSpace + 12;
  const legend = layoutLegend(model.series, plotW, legendTop, plotLeft + plotW / 2);
  const height =
    plotBottom + xLabelBand + xTitleSpace + (legend.height > 0 ? 12 + legend.height : 0) + PAD;

  const ySpan = yInfo.max - yInfo.min;
  const yScale = (v: number): number => {
    const clamped = Math.max(yInfo.min, Math.min(yInfo.max, v));
    return plotBottom - ((clamped - yInfo.min) / ySpan) * plotH;
  };
  const xCenter = (index: number): number => {
    if (xInfo.numericX) {
      return slotCount <= 1 ? plotLeft + plotW / 2 : plotLeft + ((index + 0.5) / slotCount) * plotW;
    }
    return plotLeft + (index + 0.5) * slotW;
  };

  let body = "";
  if (titleText) {
    body += text(width / 2, PAD + 10, titleText, {
      size: FONT.title,
      weight: "600",
      anchor: "middle",
    });
  }

  // Y gridlines and ticks.
  yInfo.ticks.forEach((tv, i) => {
    const ty = yScale(tv);
    body += line(plotLeft, ty, plotRight, ty, {
      stroke: color.innerStroke,
      "stroke-width": 1,
      "stroke-dasharray": "2 4",
    });
    body += line(plotLeft - TICK_LEN, ty, plotLeft, ty, { stroke: color.line, "stroke-width": 1 });
    body += text(plotLeft - TICK_LEN - 4, ty, yTickLabels[i] ?? "", {
      size: FONT.small,
      anchor: "end",
      fill: color.textSec,
    });
  });

  // Axes lines.
  body += line(plotLeft, plotTop, plotLeft, plotBottom, {
    stroke: color.line,
    "stroke-width": 1.5,
  });
  body += line(plotLeft, plotBottom, plotRight, plotBottom, {
    stroke: color.line,
    "stroke-width": 1.5,
  });

  // X ticks and labels.
  if (xInfo.numericX) {
    const xSpan = xInfo.numericX.max - xInfo.numericX.min;
    const xMin = xInfo.numericX.min;
    xInfo.numericX.ticks.forEach((tv, i) => {
      const tx = plotLeft + ((tv - xMin) / xSpan) * plotW;
      body += line(tx, plotBottom, tx, plotBottom + TICK_LEN, {
        stroke: color.line,
        "stroke-width": 1,
      });
      body += text(tx, plotBottom + TICK_LEN + 10, xLabels[i] ?? "", {
        size: FONT.small,
        anchor: "middle",
        fill: color.textSec,
      });
    });
  } else {
    xLabels.forEach((lbl, i) => {
      const tx = xCenter(i);
      body += line(tx, plotBottom, tx, plotBottom + TICK_LEN, {
        stroke: color.line,
        "stroke-width": 1,
      });
      if (rotateX) {
        body += g(
          {
            transform: `translate(${String(Math.round(tx))},${String(Math.round(plotBottom + TICK_LEN + 8))}) rotate(-35)`,
          },
          text(0, 0, lbl, { size: FONT.small, anchor: "end", fill: color.textSec }),
        );
      } else {
        body += text(tx, plotBottom + TICK_LEN + 10, lbl, {
          size: FONT.small,
          anchor: "middle",
          fill: color.textSec,
        });
      }
    });
  }

  // Axis titles.
  if (model.xAxis.title) {
    body += text(
      plotLeft + plotW / 2,
      plotBottom + xLabelBand + 12,
      fitWidth(clampLabel(model.xAxis.title), plotW, styles.axisTitleStyle),
      { size: FONT.body, weight: "600", anchor: "middle", fill: color.textSec },
    );
  }
  if (model.yAxis.title) {
    body += g(
      {
        transform: `translate(${String(PAD + 8)},${String(Math.round(plotTop + plotH / 2))}) rotate(-90)`,
      },
      text(0, 0, fitWidth(clampLabel(model.yAxis.title), plotH, styles.axisTitleStyle), {
        size: FONT.body,
        weight: "600",
        anchor: "middle",
        fill: color.textSec,
      }),
    );
  }

  // Grouped bars and line series.
  const barSeriesCount = model.series.filter((s) => s.kind === "bar").length;
  const zeroY = yScale(0);
  let barIdx = 0;
  model.series.forEach((s, plotIdx) => {
    const stroke = seriesColor(plotIdx);
    if (s.kind === "bar") {
      const groupW = Math.min(slotW * 0.72, barSeriesCount * 36);
      const singleBarW = Math.max(
        2,
        (groupW - Math.max(0, barSeriesCount - 1) * 2) / barSeriesCount,
      );
      const offset = -groupW / 2 + barIdx * (singleBarW + 2);
      barIdx++;
      const barsMarkup = s.points
        .map((pt, i) => {
          const bx = xCenter(i) + offset;
          const py = yScale(pt.value);
          const topY = Math.min(py, zeroY);
          const barH = Math.max(2, Math.abs(zeroY - py));
          return rect(bx, topY, singleBarW, barH, {
            rx: 2,
            fill: stroke,
            "fill-opacity": 0.85,
          });
        })
        .join("");
      body += g({}, barsMarkup);
    } else {
      const coords: [number, number][] = s.points.map((pt, i) => [xCenter(i), yScale(pt.value)]);
      let lineMarkup = polyline(coords, { stroke, "stroke-width": 2.2 });
      s.points.forEach((pt, i) => {
        const [px, py] = coords[i] ?? [0, 0];
        lineMarkup += circle(px, py, 3.5, {
          fill: stroke,
          stroke: color.canvas,
          "stroke-width": 1.2,
        });
        if (pt.label) {
          lineMarkup += text(px, py - 11, fitWidth(clampLabel(pt.label), 120, { size: 12 }), {
            size: 12,
            weight: "500",
            anchor: "middle",
            fill: stroke,
          });
        }
      });
      body += g({}, lineMarkup);
    }
  });

  body += drawLegendItems(legend.items);
  return svgDoc(width, height, body);
}

function drawHorizontal(
  model: XyChartModel,
  yInfo: { min: number; max: number; ticks: number[] },
  xInfo: { categories: string[]; numericX?: { min: number; max: number; ticks: number[] } },
  slotCount: number,
  styles: DrawStyles,
): string {
  const plotW = 500;
  const plotH = Math.max(300, Math.min(2400, slotCount * 28));
  const xLabels = xInfo.numericX
    ? xInfo.numericX.ticks.map(formatNum)
    : xInfo.categories.map((c) => fitWidth(clampLabel(c), 140, styles.tickStyle));
  const maxXLabelW = Math.max(24, ...xLabels.map((l) => textWidth(l, styles.tickStyle)));
  const xTitleSpace = model.xAxis.title ? 24 : 0;
  const plotLeft = PAD + xTitleSpace + maxXLabelW + 12;

  const yTickLabels = yInfo.ticks.map(formatNum);
  const yTitleSpace = model.yAxis.title ? 24 : 0;
  const rightLabelRoom = styles.hasLinePointLabels ? 128 : 0;

  const titleText = model.title
    ? fitWidth(clampLabel(model.title), plotLeft + plotW + rightLabelRoom, styles.titleStyle)
    : "";
  const titleH = titleText ? 34 : 0;
  const plotTop = PAD + titleH;
  const plotRight = plotLeft + plotW;
  const plotBottom = plotTop + plotH;
  const width = plotRight + rightLabelRoom + PAD;
  const legendTop = plotBottom + 26 + yTitleSpace + 12;
  const legend = layoutLegend(model.series, plotW, legendTop, plotLeft + plotW / 2);
  const height = plotBottom + 26 + yTitleSpace + (legend.height > 0 ? 12 + legend.height : 0) + PAD;

  const ySpan = yInfo.max - yInfo.min;
  const xScale = (v: number): number => {
    const clamped = Math.max(yInfo.min, Math.min(yInfo.max, v));
    return plotLeft + ((clamped - yInfo.min) / ySpan) * plotW;
  };
  const slotH = plotH / Math.max(1, slotCount);
  const yCenter = (index: number): number => plotTop + (index + 0.5) * slotH;

  let body = "";
  if (titleText) {
    body += text(width / 2, PAD + 10, titleText, {
      size: FONT.title,
      weight: "600",
      anchor: "middle",
    });
  }

  // Vertical gridlines for numeric Y values along bottom.
  yInfo.ticks.forEach((tv, i) => {
    const tx = xScale(tv);
    body += line(tx, plotTop, tx, plotBottom, {
      stroke: color.innerStroke,
      "stroke-width": 1,
      "stroke-dasharray": "2 4",
    });
    body += line(tx, plotBottom, tx, plotBottom + TICK_LEN, {
      stroke: color.line,
      "stroke-width": 1,
    });
    body += text(tx, plotBottom + TICK_LEN + 10, yTickLabels[i] ?? "", {
      size: FONT.small,
      anchor: "middle",
      fill: color.textSec,
    });
  });

  // Axes lines.
  body += line(plotLeft, plotTop, plotLeft, plotBottom, {
    stroke: color.line,
    "stroke-width": 1.5,
  });
  body += line(plotLeft, plotBottom, plotRight, plotBottom, {
    stroke: color.line,
    "stroke-width": 1.5,
  });

  // Left axis ticks & labels.
  if (xInfo.numericX) {
    const xSpan = xInfo.numericX.max - xInfo.numericX.min;
    const xMin = xInfo.numericX.min;
    xInfo.numericX.ticks.forEach((tv, i) => {
      const ty = plotTop + ((tv - xMin) / xSpan) * plotH;
      body += line(plotLeft - TICK_LEN, ty, plotLeft, ty, {
        stroke: color.line,
        "stroke-width": 1,
      });
      body += text(plotLeft - TICK_LEN - 4, ty, xLabels[i] ?? "", {
        size: FONT.small,
        anchor: "end",
        fill: color.textSec,
      });
    });
  } else {
    xLabels.forEach((lbl, i) => {
      const ty = yCenter(i);
      body += line(plotLeft - TICK_LEN, ty, plotLeft, ty, {
        stroke: color.line,
        "stroke-width": 1,
      });
      body += text(plotLeft - TICK_LEN - 4, ty, lbl, {
        size: FONT.small,
        anchor: "end",
        fill: color.textSec,
      });
    });
  }

  // Axis titles.
  if (model.yAxis.title) {
    body += text(
      plotLeft + plotW / 2,
      plotBottom + 26 + 12,
      fitWidth(clampLabel(model.yAxis.title), plotW, styles.axisTitleStyle),
      { size: FONT.body, weight: "600", anchor: "middle", fill: color.textSec },
    );
  }
  if (model.xAxis.title) {
    body += g(
      {
        transform: `translate(${String(PAD + 8)},${String(Math.round(plotTop + plotH / 2))}) rotate(-90)`,
      },
      text(0, 0, fitWidth(clampLabel(model.xAxis.title), plotH, styles.axisTitleStyle), {
        size: FONT.body,
        weight: "600",
        anchor: "middle",
        fill: color.textSec,
      }),
    );
  }

  // Grouped horizontal bars and line series.
  const barSeriesCount = model.series.filter((s) => s.kind === "bar").length;
  const zeroX = xScale(0);
  let barIdx = 0;
  model.series.forEach((s, plotIdx) => {
    const stroke = seriesColor(plotIdx);
    if (s.kind === "bar") {
      const groupH = Math.min(slotH * 0.72, barSeriesCount * 24);
      const singleBarH = Math.max(
        2,
        (groupH - Math.max(0, barSeriesCount - 1) * 2) / barSeriesCount,
      );
      const offset = -groupH / 2 + barIdx * (singleBarH + 2);
      barIdx++;
      const barsMarkup = s.points
        .map((pt, i) => {
          const by = yCenter(i) + offset;
          const px = xScale(pt.value);
          const leftX = Math.min(px, zeroX);
          const barW = Math.max(2, Math.abs(px - zeroX));
          return rect(leftX, by, barW, singleBarH, {
            rx: 2,
            fill: stroke,
            "fill-opacity": 0.85,
          });
        })
        .join("");
      body += g({}, barsMarkup);
    } else {
      const coords: [number, number][] = s.points.map((pt, i) => [xScale(pt.value), yCenter(i)]);
      let lineMarkup = polyline(coords, { stroke, "stroke-width": 2.2 });
      s.points.forEach((pt, i) => {
        const [px, py] = coords[i] ?? [0, 0];
        lineMarkup += circle(px, py, 3.5, {
          fill: stroke,
          stroke: color.canvas,
          "stroke-width": 1.2,
        });
        if (pt.label) {
          lineMarkup += text(px + 8, py, fitWidth(clampLabel(pt.label), 120, { size: 12 }), {
            size: 12,
            weight: "500",
            anchor: "start",
            fill: stroke,
          });
        }
      });
      body += g({}, lineMarkup);
    }
  });

  body += drawLegendItems(legend.items);
  return svgDoc(width, height, body);
}
