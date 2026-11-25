/**
 * C4 diagram drawing: nested boundary/deployment-node grid layout honoring `c4ShapeInRow` and
 * `c4BoundaryInRow`, element/boundary shape drawing, curved/orthogonal relationship routing with
 * parallel spread, dynamic sequence badges, and UpdateRelStyle label offsets.
 */
import { type Point } from "../../layout/layered";
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import { el, markerDefs, markerRef, n, path, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color } from "../../theme";
import type { C4Element, C4Item, C4Model, C4RelDir, C4Scope } from "./parse";
import { drawC4Element, drawC4ScopeFrame, measureC4Element, type ElementMetrics } from "./shapes";

const SHAPE_GAP_X = 58;
const SHAPE_GAP_Y = 56;
const SCOPE_GAP_X = 48;
const SCOPE_GAP_Y = 48;
const RUN_GAP = 42;
const SCOPE_PAD = 22;
const BOUNDARY_HDR = 42;
const NODE_HDR = 44;
const NODE_DESCR_HDR = 56;
const MARGIN = 24;

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

type RunPlacement =
  | {
      kind: "element";
      item: C4Element;
      metrics: ElementMetrics;
      x: number;
      y: number;
      width: number;
      height: number;
    }
  | {
      kind: "scope";
      item: C4Scope;
      laid: LaidScope;
      x: number;
      y: number;
      width: number;
      height: number;
    };

interface LaidScope {
  width: number;
  height: number;
  placements: RunPlacement[];
}

function scopeHeaderHeight(scope: C4Scope | undefined): number {
  if (!scope) return 0;
  if (scope.scopeType === "boundary") return BOUNDARY_HDR;
  return scope.descr ? NODE_DESCR_HDR : NODE_HDR;
}

function layoutGrid(
  items: { width: number; height: number; payload: RunPlacement }[],
  perRow: number,
  gapX: number,
  gapY: number,
): {
  width: number;
  height: number;
  rows: { items: RunPlacement[]; width: number; height: number }[];
} {
  const rows: { items: RunPlacement[]; width: number; height: number }[] = [];
  for (let i = 0; i < items.length; i += perRow) {
    const slice = items.slice(i, i + perRow);
    const width =
      slice.reduce((sum, entry) => sum + entry.width, 0) + Math.max(0, slice.length - 1) * gapX;
    const height = Math.max(0, ...slice.map((entry) => entry.height));
    rows.push({
      items: slice.map((entry) => entry.payload),
      width,
      height,
    });
  }
  const maxWidth = Math.max(0, ...rows.map((r) => r.width));
  const totalHeight =
    rows.reduce((sum, r) => sum + r.height, 0) + Math.max(0, rows.length - 1) * gapY;
  return { width: maxWidth, height: totalHeight, rows };
}

function layoutItems(
  items: readonly C4Item[],
  scope: C4Scope | undefined,
  model: C4Model,
  budget: StepBudget,
): LaidScope {
  budget.tick();
  const isRoot = scope === undefined;
  const padX = isRoot ? 0 : SCOPE_PAD;
  const hdrH = scopeHeaderHeight(scope);
  const padBot = isRoot ? 0 : SCOPE_PAD;

  // Partition children into consecutive element runs and scope runs preserving document order.
  const runs: { kind: "element" | "scope"; list: C4Item[] }[] = [];
  for (const child of items) {
    const last = runs[runs.length - 1];
    if (last && last.kind === child.kind) last.list.push(child);
    else runs.push({ kind: child.kind, list: [child] });
  }

  const laidRuns: {
    gapY: number;
    gapX: number;
    rows: { items: RunPlacement[]; width: number; height: number }[];
    width: number;
    height: number;
  }[] = [];

  for (const run of runs) {
    budget.tick();
    if (run.kind === "element") {
      const prepared = run.list.map((raw) => {
        const item = raw as C4Element;
        const metrics = measureC4Element(item);
        const payload: RunPlacement = {
          kind: "element",
          item,
          metrics,
          x: 0,
          y: 0,
          width: metrics.width,
          height: metrics.height,
        };
        return { width: metrics.width, height: metrics.height, payload };
      });
      const grid = layoutGrid(prepared, model.shapeInRow, SHAPE_GAP_X, SHAPE_GAP_Y);
      laidRuns.push({ ...grid, gapX: SHAPE_GAP_X, gapY: SHAPE_GAP_Y });
    } else {
      const prepared = run.list.map((raw) => {
        const childScope = raw as C4Scope;
        const laid = layoutItems(childScope.children, childScope, model, budget);
        const payload: RunPlacement = {
          kind: "scope",
          item: childScope,
          laid,
          x: 0,
          y: 0,
          width: laid.width,
          height: laid.height,
        };
        return { width: laid.width, height: laid.height, payload };
      });
      const grid = layoutGrid(prepared, model.boundaryInRow, SCOPE_GAP_X, SCOPE_GAP_Y);
      laidRuns.push({ ...grid, gapX: SCOPE_GAP_X, gapY: SCOPE_GAP_Y });
    }
  }

  const headerMinW = scope
    ? Math.max(
        textWidth(clampLabel(scope.label), { size: FONT.body, bold: true }) + 40,
        scope.typeLabel
          ? textWidth(`[${clampLabel(scope.typeLabel)}]`, { size: FONT.small }) + 40
          : 0,
        160,
      )
    : 0;

  const contentW = Math.max(headerMinW, ...laidRuns.map((r) => r.width));

  const placements: RunPlacement[] = [];
  let yCursor = hdrH;
  laidRuns.forEach((run, runIdx) => {
    if (runIdx > 0) yCursor += RUN_GAP;
    run.rows.forEach((row, rowIdx) => {
      if (rowIdx > 0) yCursor += run.gapY;
      let xCursor = padX + (contentW - row.width) / 2;
      for (const entry of row.items) {
        entry.x = xCursor;
        entry.y = yCursor + (row.height - entry.height) / 2;
        placements.push(entry);
        xCursor += entry.width + run.gapX;
      }
      yCursor += row.height;
    });
  });

  return {
    width: contentW + padX * 2,
    height: Math.max(hdrH + padBot + (items.length === 0 ? 36 : 0), yCursor + padBot),
    placements,
  };
}

function choosePoints(a: Box, b: Box, hint: C4RelDir, offsetIndex: number): Point[] {
  const acx = a.x + a.width / 2;
  const acy = a.y + a.height / 2;
  const bcx = b.x + b.width / 2;
  const bcy = b.y + b.height / 2;
  const dx = bcx - acx;
  const dy = bcy - acy;

  let p0: Point;
  let p1: Point;

  if (hint === "L" || (hint === "none" && Math.abs(dx) > Math.abs(dy) && dx < 0)) {
    p0 = [a.x, acy];
    p1 = [b.x + b.width, bcy];
  } else if (hint === "R" || (hint === "none" && Math.abs(dx) > Math.abs(dy) && dx >= 0)) {
    p0 = [a.x + a.width, acy];
    p1 = [b.x, bcy];
  } else if (hint === "U" || (hint === "none" && dy < 0)) {
    p0 = [acx, a.y];
    p1 = [bcx, b.y + b.height];
  } else {
    p0 = [acx, a.y + a.height];
    p1 = [bcx, b.y];
  }

  // Spread parallel/opposite edges perpendicular to the segment.
  if (offsetIndex !== 0) {
    const len = Math.hypot(p1[0] - p0[0], p1[1] - p0[1]) || 1;
    const nx = -(p1[1] - p0[1]) / len;
    const ny = (p1[0] - p0[0]) / len;
    const shift = offsetIndex * 14;
    p0 = [p0[0] + nx * shift, p0[1] + ny * shift];
    p1 = [p1[0] + nx * shift, p1[1] + ny * shift];
  }
  return [p0, p1];
}

/** Draw a parsed C4 diagram as a complete passive SVG string. */
export function drawC4(model: C4Model): string {
  const budget = new StepBudget();
  const rootLaid = layoutItems(model.root.items, undefined, model, budget);
  const titleText = model.title
    ? fitWidth(clampLabel(model.title), Math.max(420, rootLaid.width), {
        size: FONT.title,
        bold: true,
      })
    : "";
  const titleH = titleText ? 36 : 0;

  const boxes = new Map<string, Box>();
  const back: string[] = [];
  const front: string[] = [];

  const emit = (laid: LaidScope, ox: number, oy: number): void => {
    for (const p of laid.placements) {
      budget.tick();
      const px = ox + p.x;
      const py = oy + p.y;
      boxes.set(p.item.id, { x: px, y: py, width: p.width, height: p.height });
      if (p.kind === "element") {
        front.push(drawC4Element(p.item, p.metrics, px, py));
      } else {
        back.push(drawC4ScopeFrame(p.item, px, py, p.width, p.height));
        emit(p.laid, px, py);
      }
    }
  };

  const baseOx = MARGIN;
  const baseOy = MARGIN + titleH;
  emit(rootLaid, baseOx, baseOy);

  let minX = 0;
  let minY = 0;
  let maxX = Math.max(
    rootLaid.width + MARGIN * 2,
    titleText ? textWidth(titleText, { size: FONT.title, bold: true }) + MARGIN * 2 : 0,
  );
  let maxY = rootLaid.height + titleH + MARGIN * 2;

  const pairCounts = new Map<string, number>();
  const pairSeen = new Map<string, number>();
  for (const rel of model.relations) {
    const key = [rel.from, rel.to].sort().join("\u0000");
    pairCounts.set(key, (pairCounts.get(key) ?? 0) + 1);
  }

  const edgesSvg: string[] = [];
  const labelsSvg: string[] = [];

  for (const rel of model.relations) {
    budget.tick();
    const a = boxes.get(rel.from);
    const b = boxes.get(rel.to);
    if (!a || !b) continue;

    const key = [rel.from, rel.to].sort().join("\u0000");
    const total = pairCounts.get(key) ?? 1;
    const seen = pairSeen.get(key) ?? 0;
    pairSeen.set(key, seen + 1);
    const spreadIdx = total > 1 ? seen - (total - 1) / 2 : 0;

    let d: string;
    let labelCx: number;
    let labelCy: number;

    if (rel.from === rel.to) {
      const rx = a.x + a.width;
      const cy = a.y + a.height / 2;
      d = `M${n(rx)},${n(cy - 12)} L${n(rx + 36)},${n(cy - 12)} L${n(rx + 36)},${n(cy + 12)} L${n(rx)},${n(cy + 12)}`;
      labelCx = rx + 64;
      labelCy = cy;
    } else {
      const pts = choosePoints(a, b, rel.dir, spreadIdx);
      const p0 = pts[0] as Point;
      const p1 = pts[1] as Point;
      const len = Math.hypot(p1[0] - p0[0], p1[1] - p0[1]) || 1;
      const nx = -(p1[1] - p0[1]) / len;
      const ny = (p1[0] - p0[0]) / len;
      const bow = spreadIdx !== 0 ? spreadIdx * 24 : 0;
      const ctrlX = (p0[0] + p1[0]) / 2 + nx * bow;
      const ctrlY = (p0[1] + p1[1]) / 2 + ny * bow;
      d =
        bow === 0
          ? `M${n(p0[0])},${n(p0[1])} L${n(p1[0])},${n(p1[1])}`
          : `M${n(p0[0])},${n(p0[1])} Q${n(ctrlX)},${n(ctrlY)} ${n(p1[0])},${n(p1[1])}`;
      labelCx = 0.25 * p0[0] + 0.5 * ctrlX + 0.25 * p1[0];
      labelCy = 0.25 * p0[1] + 0.5 * ctrlY + 0.25 * p1[1];
    }

    edgesSvg.push(
      path(d, {
        fill: "none",
        stroke: color.line,
        "stroke-width": 1.4,
        "marker-start": rel.fromArrow ? markerRef("arrow-filled") : undefined,
        "marker-end": rel.toArrow ? markerRef("arrow-filled") : undefined,
      }),
    );

    const offset = model.relOffsets.get(`${rel.from}->${rel.to}`);
    if (offset) {
      labelCx += offset.x;
      labelCy += offset.y;
    }

    const prefix = rel.sequence !== undefined ? `${String(rel.sequence)}: ` : "";
    const mainText = `${prefix}${clampLabel(rel.label)}`;
    const mainLines = mainText.trim()
      ? wrapText(mainText, 160, { size: FONT.small + 1, bold: true }, 2)
      : [];
    const technLine = rel.techn
      ? `[${fitWidth(clampLabel(rel.techn), 160, { size: FONT.small })}]`
      : "";

    if (mainLines.length > 0 || technLine) {
      const mSize =
        mainLines.length > 0
          ? measureLines(mainLines, { size: FONT.small + 1, bold: true })
          : { width: 0, height: 0 };
      const tW = technLine ? textWidth(technLine, { size: FONT.small }) : 0;
      const pillW = Math.max(mSize.width, tW) + 14;
      const pillH = mainLines.length * 15 + (technLine ? 14 : 0) + 8;
      const px = labelCx - pillW / 2;
      const py = labelCy - pillH / 2;

      minX = Math.min(minX, px - MARGIN);
      minY = Math.min(minY, py - MARGIN);
      maxX = Math.max(maxX, px + pillW + MARGIN);
      maxY = Math.max(maxY, py + pillH + MARGIN);

      labelsSvg.push(
        rect(px, py, pillW, pillH, {
          rx: 6,
          fill: color.canvas,
          "fill-opacity": 0.94,
          stroke: color.innerStroke,
          "stroke-width": 0.8,
        }),
      );
      let ty = py + 11;
      if (mainLines.length > 0) {
        labelsSvg.push(
          textLines(
            labelCx,
            ty,
            mainLines,
            { size: FONT.small + 1, weight: "500", anchor: "middle" },
            15 / (FONT.small + 1),
          ),
        );
        ty += mainLines.length * 15;
      }
      if (technLine) {
        labelsSvg.push(
          text(labelCx, ty, technLine, {
            size: FONT.small,
            anchor: "middle",
            fill: color.textSec,
          }),
        );
      }
    }
  }

  const titleSvg = titleText
    ? text((minX + maxX) / 2, MARGIN + 10, titleText, {
        size: FONT.title,
        weight: "600",
        anchor: "middle",
      })
    : "";

  const defs = markerDefs(["arrow-filled"]);
  const body = el(
    "g",
    { transform: `translate(${n(-minX)},${n(-minY)})` },
    [titleSvg, ...back, ...front, ...edgesSvg, ...labelsSvg].join(""),
  );
  return svgDoc(maxX - minX, maxY - minY, body, defs);
}
