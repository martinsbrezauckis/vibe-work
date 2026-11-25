/**
 * Event Modeling diagram drawing: horizontal timeline swimlanes ordered by first appearance,
 * frame cards (number, entity-type pill, name, inline/`data` snippets), and inferred or explicit
 * orthogonal connector arrows.
 */
import { StepBudget } from "../../limits";
import { fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import { line, markerDefs, markerRef, n, path, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { EmEntityType, EmFrame, EventModelingModel } from "./parse";

const PAD = 20;
const MIN_LANE_HEAD_W = 180;
const MAX_LANE_HEAD_W = 240;
const COL_GAP = 34;
const ROW_PAD = 18;
const CARD_PAD = 12;
const LINE_H = 16;

const TYPE_STYLE: Record<EmEntityType, { badge: string; tint: string }> = {
  ui: { badge: "UI", tint: series(1) },
  pcr: { badge: "PCR", tint: series(6) },
  cmd: { badge: "CMD", tint: series(0) },
  rmo: { badge: "RMO", tint: series(4) },
  evt: { badge: "EVT", tint: series(3) },
};

interface CardBox {
  frame: EmFrame;
  width: number;
  height: number;
  titleLines: string[];
  dataLines: string[];
}

function dataLinesFor(frame: EmFrame, model: EventModelingModel): string[] {
  const rawLines: string[] = [];
  if (frame.inlineData !== undefined) {
    rawLines.push(frame.inlineData);
  } else if (frame.dataRef !== undefined) {
    const block = model.dataBlocks.get(frame.dataRef);
    if (block) rawLines.push(...block.fields);
  }
  const wrapped: string[] = [];
  const style = { size: FONT.small };
  for (const raw of rawLines.slice(0, 4)) {
    wrapped.push(...wrapText(raw, 176, style, 2));
  }
  if (rawLines.length > 4) wrapped.push(`+${String(rawLines.length - 4)} more`);
  return wrapped.slice(0, 5);
}

function buildCard(frame: EmFrame, model: EventModelingModel): CardBox {
  const titleStyle = { size: FONT.body, bold: true };
  const titleLines = wrapText(frame.displayName, 176, titleStyle, 2);
  const dataLines = dataLinesFor(frame, model);
  const headerW =
    textWidth(`#${frame.num}`, { size: FONT.small, bold: true }) +
    textWidth(TYPE_STYLE[frame.type].badge, { size: FONT.small, bold: true }) +
    (frame.isReset ? 30 : 0) +
    36;
  const titleW = measureLines(titleLines, titleStyle).width + CARD_PAD * 2;
  const dataW = dataLines.length
    ? measureLines(dataLines, { size: FONT.small }).width + CARD_PAD * 2
    : 0;
  const width = Math.min(220, Math.max(136, headerW, titleW, dataW));
  const titleH = titleLines.length * 18;
  const dataH = dataLines.length ? dataLines.length * LINE_H + 12 : 0;
  return {
    frame,
    width,
    height: 30 + titleH + dataH + 10,
    titleLines,
    dataLines,
  };
}

/** Draw a parsed Event Modeling diagram as a complete SVG string. */
export function drawEventModeling(model: EventModelingModel): string {
  const budget = new StepBudget();
  const cards = model.frames.map((frame) => {
    budget.tick();
    return buildCard(frame, model);
  });

  const laneIndex = new Map(model.lanes.map((lane, i) => [lane.id, i]));
  const laneHeights = model.lanes.map(() => 84);
  cards.forEach((card) => {
    const idx = laneIndex.get(card.frame.laneId) ?? 0;
    laneHeights[idx] = Math.max(laneHeights[idx] ?? 84, card.height + ROW_PAD * 2);
  });

  const laneTops: number[] = [];
  let yCursor = PAD;
  laneHeights.forEach((h) => {
    laneTops.push(yCursor);
    yCursor += h + 10;
  });

  const laneHeadW = Math.min(
    MAX_LANE_HEAD_W,
    Math.max(
      MIN_LANE_HEAD_W,
      ...model.lanes.map((lane) => textWidth(lane.label, { size: FONT.body, bold: true }) + 28),
    ),
  );
  const colCenters: number[] = [];
  let xCursor = PAD + laneHeadW + 24;
  cards.forEach((card) => {
    colCenters.push(xCursor + card.width / 2);
    xCursor += card.width + COL_GAP;
  });

  const totalW = Math.max(480, xCursor - COL_GAP + 24 + PAD);
  const totalH = yCursor - 10 + PAD;

  const back: string[] = [];
  const mid: string[] = [];
  const front: string[] = [];

  // Swimlane horizontal bands.
  model.lanes.forEach((lane, i) => {
    budget.tick();
    const ly = laneTops[i] ?? PAD;
    const lh = laneHeights[i] ?? 84;
    back.push(
      rect(PAD, ly, totalW - PAD * 2, lh, {
        rx: 8,
        fill: series(i),
        "fill-opacity": 0.06,
        stroke: color.innerStroke,
        "stroke-width": 1,
      }),
    );
    back.push(
      rect(PAD, ly, laneHeadW, lh, {
        rx: 8,
        fill: color.groupHdr,
        "fill-opacity": 0.8,
        stroke: "none",
      }),
    );
    back.push(
      line(PAD + laneHeadW, ly, PAD + laneHeadW, ly + lh, {
        stroke: color.innerStroke,
        "stroke-width": 1,
      }),
    );
    const labelLines = wrapText(lane.label, laneHeadW - 22, { size: FONT.body, bold: true }, 3);
    front.push(
      textLines(
        PAD + 14,
        ly + lh / 2 - ((labelLines.length - 1) * 17) / 2,
        labelLines,
        { size: FONT.body, weight: "600", fill: color.text },
        17 / FONT.body,
      ),
    );
  });

  // Placed card centers for edge routing.
  const placed = new Map<
    string,
    { cx: number; cy: number; w: number; h: number; col: number; row: number }
  >();
  cards.forEach((card, col) => {
    const row = laneIndex.get(card.frame.laneId) ?? 0;
    const cx = colCenters[col] ?? 0;
    const cy = (laneTops[row] ?? PAD) + (laneHeights[row] ?? 84) / 2;
    placed.set(card.frame.num, { cx, cy, w: card.width, h: card.height, col, row });
  });

  // Edges routed between card right/left or top/bottom borders.
  model.edges.forEach((edge, idx) => {
    budget.tick();
    const a = placed.get(edge.from);
    const b = placed.get(edge.to);
    if (!a || !b) return;
    const sameRow = a.row === b.row;
    const adjacent = b.col === a.col + 1;
    let d: string;
    if (adjacent) {
      const sx = a.cx + a.w / 2;
      const sy = a.cy;
      const ex = b.cx - b.w / 2;
      const ey = b.cy;
      if (sameRow) {
        d = `M${n(sx)},${n(sy)} L${n(ex)},${n(ey)}`;
      } else {
        const mx = (sx + ex) / 2;
        d = `M${n(sx)},${n(sy)} L${n(mx)},${n(sy)} L${n(mx)},${n(ey)} L${n(ex)},${n(ey)}`;
      }
    } else {
      // Explicit long or backward relation: step above or below source card into target top/bottom edge.
      const dirY = b.row >= a.row ? 1 : -1;
      const sy = a.cy + dirY * (a.h / 2);
      const ey = b.cy - dirY * (b.h / 2);
      const sx = a.cx + ((idx % 3) - 1) * 8;
      const ex = b.cx + ((idx % 3) - 1) * 8;
      const laneGapY = sameRow
        ? a.cy - a.h / 2 - 12 - (idx % 3) * 5
        : (sy + ey) / 2 + ((idx % 3) - 1) * 4;
      const startExitY = sameRow ? laneGapY : laneGapY;
      d = sameRow
        ? `M${n(sx)},${n(a.cy - a.h / 2)} L${n(sx)},${n(startExitY)} L${n(ex)},${n(startExitY)} L${n(ex)},${n(b.cy - b.h / 2)}`
        : `M${n(sx)},${n(sy)} L${n(sx)},${n(laneGapY)} L${n(ex)},${n(laneGapY)} L${n(ex)},${n(ey)}`;
    }
    mid.push(
      path(d, {
        fill: "none",
        stroke: edge.inferred ? color.line : color.arrow,
        "stroke-width": edge.inferred ? 1.3 : 1.5,
        "stroke-dasharray": edge.inferred ? undefined : "5 3",
        "marker-end": markerRef("arrow-filled"),
      }),
    );
  });

  // Draw cards on top.
  cards.forEach((card) => {
    budget.tick();
    const pos = placed.get(card.frame.num);
    if (!pos) return;
    const x = pos.cx - card.width / 2;
    const y = pos.cy - card.height / 2;
    const meta = TYPE_STYLE[card.frame.type];

    front.push(
      rect(x, y, card.width, card.height, {
        rx: 7,
        fill: color.nodeFill,
        stroke: card.frame.isReset ? color.warn : meta.tint,
        "stroke-width": card.frame.isReset ? 1.6 : 1.3,
        "stroke-dasharray": card.frame.isReset ? "5 3" : undefined,
      }),
    );
    front.push(
      rect(x + 1, y + 1, card.width - 2, 24, {
        rx: 6,
        fill: meta.tint,
        "fill-opacity": 0.16,
      }),
    );
    front.push(
      text(x + 10, y + 13, `#${card.frame.num}`, {
        size: FONT.small,
        weight: "700",
        fill: color.textSec,
      }),
    );
    if (card.frame.isReset) {
      front.push(
        text(x + 40, y + 13, "RF", {
          size: FONT.small,
          weight: "700",
          fill: color.warn,
        }),
      );
    }
    const badgeW = textWidth(meta.badge, { size: FONT.small, bold: true }) + 12;
    front.push(
      rect(x + card.width - badgeW - 8, y + 5, badgeW, 16, {
        rx: 4,
        fill: meta.tint,
        "fill-opacity": 0.24,
      }),
    );
    front.push(
      text(x + card.width - 8 - badgeW / 2, y + 13, meta.badge, {
        size: FONT.small,
        weight: "700",
        anchor: "middle",
        fill: color.text,
      }),
    );

    front.push(
      textLines(
        pos.cx,
        y + 38,
        card.titleLines,
        { size: FONT.body, weight: "600", anchor: "middle" },
        18 / FONT.body,
      ),
    );

    if (card.dataLines.length > 0) {
      const dataY = y + 32 + card.titleLines.length * 18;
      front.push(
        line(x + 8, dataY, x + card.width - 8, dataY, {
          stroke: color.innerStroke,
          "stroke-width": 1,
        }),
      );
      card.dataLines.forEach((dLine, di) => {
        front.push(
          text(
            x + 10,
            dataY + 12 + di * LINE_H,
            fitWidth(dLine, card.width - 20, { size: FONT.small }),
            { size: FONT.small, fill: color.textMuted },
          ),
        );
      });
    }
  });

  return svgDoc(totalW, totalH, [...back, ...mid, ...front].join(""), markerDefs(["arrow-filled"]));
}
