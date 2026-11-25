/**
 * Block diagram drawing: bottom-up grid track sizing with column spans and nested composites,
 * top-down placement, all 13 node shapes + directional block arrows, and orthogonal port-to-port
 * connectors with label pills.
 */
import { roundedPath, type Point } from "../../layout/layered";
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import { el, markerDefs, markerRef, n, path, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import { DiagramLimitError } from "../../types";
import { MAX_BLOCK_COLUMNS, type BlockGroup, type BlockItem, type BlockModel } from "./parse";
import { routeEdge, type Box } from "./route";
import { drawBlockArrow, drawLeafShape, shapePadding } from "./shapes";

const CELL_GAP = 18;
const GRP_PAD = 14;
const GRP_HDR = 24;
const MARGIN = 20;
const MIN_CELL_W = 68;
const MIN_CELL_H = 42;
const LABEL_STYLE = { size: FONT.body, bold: false };

interface CellPlacement {
  item: BlockItem;
  row: number;
  col: number;
  span: number;
  measured?: MeasuredGroup | undefined;
  lines: string[];
  minW: number;
  minH: number;
}

interface MeasuredGroup {
  cols: number;
  colWidths: number[];
  rowHeights: number[];
  cells: CellPlacement[];
  width: number;
  height: number;
  hasHeader: boolean;
}

function measureGroup(group: BlockGroup, isRoot: boolean, budget: StepBudget): MeasuredGroup {
  const autoCols = group.items.reduce((sum, item) => sum + item.span, 0);
  const maxItemSpan = Math.max(1, ...group.items.map((item) => item.span));
  const cols =
    group.columns !== undefined ? Math.max(group.columns, maxItemSpan) : Math.max(1, autoCols);
  if (!Number.isSafeInteger(cols) || cols < 1 || cols > MAX_BLOCK_COLUMNS) {
    throw new DiagramLimitError(
      `Diagram too large: block columns must be a whole number from 1 to ${String(MAX_BLOCK_COLUMNS)}`,
    );
  }

  const cells: CellPlacement[] = [];
  let row = 0;
  let col = 0;

  for (const item of group.items) {
    budget.tick();
    const span = Math.min(item.span, cols);
    if (col + span > cols) {
      row++;
      col = 0;
    }
    let minW: number;
    let minH: number;
    let lines: string[] = [];
    let measured: MeasuredGroup | undefined;

    if (item.kind === "space") {
      minW = 40 * span;
      minH = 28;
    } else if (item.kind === "arrow") {
      const trimmed = clampLabel(item.label).trim();
      lines = trimmed ? wrapText(trimmed, 180, LABEL_STYLE, 2) : [];
      const size = lines.length > 0 ? measureLines(lines, LABEL_STYLE) : { width: 32, height: 18 };
      minW = Math.max(76, size.width + 44);
      minH = Math.max(44, size.height + 24);
    } else if (item.kind === "node") {
      lines = wrapText(clampLabel(item.label), 220, LABEL_STYLE, 3);
      const size = measureLines(lines, LABEL_STYLE);
      const { px, py } = shapePadding(item.shape);
      minW = Math.max(MIN_CELL_W, size.width + px * 2);
      minH = Math.max(MIN_CELL_H, size.height + py * 2);
      if (item.shape === "circle" || item.shape === "doublecircle") {
        const d = Math.max(minW, minH);
        minW = d;
        minH = d;
      }
    } else {
      measured = measureGroup(item, false, budget);
      minW = measured.width;
      minH = measured.height;
    }

    cells.push({ item, row, col, span, measured, lines, minW, minH });
    col += span;
  }

  const rows = cells.length === 0 ? 1 : Math.max(...cells.map((c) => c.row)) + 1;
  const colWidths = new Array<number>(cols).fill(MIN_CELL_W);
  const rowHeights = new Array<number>(rows).fill(MIN_CELL_H);

  // First pass: single-column items establish column floors; all items establish row floors.
  for (const cell of cells) {
    budget.tick();
    rowHeights[cell.row] = Math.max(rowHeights[cell.row] ?? MIN_CELL_H, cell.minH);
    if (cell.span === 1) {
      colWidths[cell.col] = Math.max(colWidths[cell.col] ?? MIN_CELL_W, cell.minW);
    }
  }
  // Second pass: multi-column spans widen covered columns if their current total is too narrow.
  for (const cell of cells) {
    if (cell.span <= 1) continue;
    budget.tick();
    let covered = (cell.span - 1) * CELL_GAP;
    for (let c = cell.col; c < cell.col + cell.span; c++) covered += colWidths[c] ?? 0;
    if (covered < cell.minW) {
      const add = (cell.minW - covered) / cell.span;
      for (let c = cell.col; c < cell.col + cell.span; c++) {
        colWidths[c] = (colWidths[c] ?? 0) + add;
      }
    }
  }

  const hasHeader = !isRoot && Boolean(group.id);
  const padX = isRoot ? 0 : GRP_PAD;
  const padTop = isRoot ? 0 : hasHeader ? GRP_HDR + 6 : GRP_PAD;
  const padBot = isRoot ? 0 : GRP_PAD;
  const headerW =
    hasHeader && group.id
      ? textWidth(clampLabel(group.id), { size: FONT.small + 1, bold: true }) + 28
      : 0;

  const contentW = colWidths.reduce((a, b) => a + b, 0) + Math.max(0, cols - 1) * CELL_GAP;
  const contentH = rowHeights.reduce((a, b) => a + b, 0) + Math.max(0, rows - 1) * CELL_GAP;
  return {
    cols,
    colWidths,
    rowHeights,
    cells,
    width: Math.max(contentW + padX * 2, headerW),
    height: contentH + padTop + padBot,
    hasHeader,
  };
}

/** Draw a parsed block diagram as a complete passive SVG string. */
export function drawBlock(model: BlockModel): string {
  const budget = new StepBudget();
  const rootMeasured = measureGroup(model.root, true, budget);
  const idBoxes = new Map<string, Box>();
  const back: string[] = [];
  const front: string[] = [];
  let tintIndex = 0;

  const renderGroup = (
    group: BlockGroup,
    measured: MeasuredGroup,
    ox: number,
    oy: number,
    assignedW: number,
    assignedH: number,
    isRoot: boolean,
  ): void => {
    const padX = isRoot ? 0 : GRP_PAD;
    const padTop = isRoot ? 0 : measured.hasHeader ? GRP_HDR + 6 : GRP_PAD;
    const padBot = isRoot ? 0 : GRP_PAD;

    const baseContentW =
      measured.colWidths.reduce((a, b) => a + b, 0) + Math.max(0, measured.cols - 1) * CELL_GAP;
    const baseContentH =
      measured.rowHeights.reduce((a, b) => a + b, 0) +
      Math.max(0, measured.rowHeights.length - 1) * CELL_GAP;
    const extraW = Math.max(0, assignedW - padX * 2 - baseContentW);
    const extraH = Math.max(0, assignedH - padTop - padBot - baseContentH);
    const colWidths = measured.colWidths.map((w) => w + extraW / measured.cols);
    const rowHeights = measured.rowHeights.map(
      (h) => h + extraH / Math.max(1, measured.rowHeights.length),
    );

    if (!isRoot) {
      back.push(
        rect(ox, oy, assignedW, assignedH, {
          rx: 8,
          fill: color.groupFill,
          stroke: color.innerStroke,
          "stroke-width": 1.2,
          "stroke-dasharray": "6 4",
        }),
      );
      if (group.id) {
        back.push(
          text(
            ox + 10,
            oy + 14,
            fitWidth(clampLabel(group.id), assignedW - 20, { size: FONT.small + 1, bold: true }),
            {
              size: FONT.small + 1,
              weight: "600",
              fill: color.textSec,
            },
          ),
        );
        idBoxes.set(group.id, { x: ox, y: oy, width: assignedW, height: assignedH, group: true });
      }
    }

    const colX: number[] = [];
    let cxCursor = ox + padX;
    for (const w of colWidths) {
      colX.push(cxCursor);
      cxCursor += w + CELL_GAP;
    }
    const rowY: number[] = [];
    let ryCursor = oy + padTop;
    for (const h of rowHeights) {
      rowY.push(ryCursor);
      ryCursor += h + CELL_GAP;
    }

    for (const cell of measured.cells) {
      budget.tick();
      const bx = colX[cell.col] ?? ox + padX;
      const by = rowY[cell.row] ?? oy + padTop;
      let bw = (cell.span - 1) * CELL_GAP;
      for (let c = cell.col; c < cell.col + cell.span; c++) bw += colWidths[c] ?? 0;
      const bh = rowHeights[cell.row] ?? MIN_CELL_H;

      if (cell.item.kind === "space") continue;
      if (cell.item.kind === "group" && cell.measured) {
        renderGroup(cell.item, cell.measured, bx, by, bw, bh, false);
        continue;
      }

      const tint = series(tintIndex++);
      if (cell.item.kind === "arrow") {
        idBoxes.set(cell.item.id, { x: bx, y: by, width: bw, height: bh });
        front.push(drawBlockArrow(cell.item.dirs, bx, by, bw, bh, tint));
        if (cell.lines.length > 0) {
          const step = LABEL_STYLE.size * 1.2;
          const firstY = by + bh / 2 - ((cell.lines.length - 1) * step) / 2;
          front.push(
            textLines(
              bx + bw / 2,
              firstY,
              cell.lines,
              {
                size: LABEL_STYLE.size,
                weight: "500",
                anchor: "middle",
              },
              1.2,
            ),
          );
        }
      } else if (cell.item.kind === "node") {
        idBoxes.set(cell.item.id, { x: bx, y: by, width: bw, height: bh });
        front.push(drawLeafShape(cell.item.shape, bx, by, bw, bh, tint));
        const step = LABEL_STYLE.size * 1.25;
        const firstY = by + bh / 2 - ((cell.lines.length - 1) * step) / 2;
        front.push(
          textLines(
            bx + bw / 2,
            firstY,
            cell.lines,
            {
              size: LABEL_STYLE.size,
              weight: "500",
              anchor: "middle",
            },
            1.25,
          ),
        );
      }
    }
  };

  renderGroup(
    model.root,
    rootMeasured,
    MARGIN,
    MARGIN,
    rootMeasured.width,
    rootMeasured.height,
    true,
  );

  const edgeSvg: string[] = [];
  const labelSvg: string[] = [];
  let maxX = rootMeasured.width + MARGIN * 2;
  let maxY = rootMeasured.height + MARGIN * 2;

  model.edges.forEach((edge, idx) => {
    const a = idBoxes.get(edge.from);
    const b = idBoxes.get(edge.to);
    if (!a || !b) return;
    const acy = a.y + a.height / 2;

    let pts: Point[];
    if (edge.from === edge.to) {
      const rx = a.x + a.width;
      pts = [
        [rx, acy - 10],
        [rx + 28, acy - 10],
        [rx + 28, acy + 10],
        [rx, acy + 10],
      ];
      maxX = Math.max(maxX, rx + 40);
    } else {
      pts = routeEdge(a, b, idx);
    }

    for (const [px, py] of pts) {
      maxX = Math.max(maxX, px + MARGIN);
      maxY = Math.max(maxY, py + MARGIN);
    }

    edgeSvg.push(
      path(roundedPath(pts, 7), {
        fill: "none",
        stroke: color.line,
        "stroke-width": 1.4,
        "stroke-dasharray": edge.dashed ? "6 4" : undefined,
        "marker-start": edge.fromArrow ? markerRef("arrow-filled") : undefined,
        "marker-end": edge.toArrow ? markerRef("arrow-filled") : undefined,
      }),
    );

    if (edge.label) {
      const midPt = pts[Math.floor(pts.length / 2)] ?? pts[0] ?? [0, 0];
      const prevPt = pts[Math.max(0, Math.floor(pts.length / 2) - 1)] ?? midPt;
      const lx = (midPt[0] + prevPt[0]) / 2;
      const ly = (midPt[1] + prevPt[1]) / 2;
      const lbl = clampLabel(edge.label);
      const lw = textWidth(lbl, { size: FONT.small + 1 }) + 12;
      labelSvg.push(
        rect(lx - lw / 2, ly - 10, lw, 20, {
          rx: 8,
          fill: color.canvas,
          stroke: color.innerStroke,
          "stroke-width": 0.8,
        }),
      );
      labelSvg.push(
        text(lx, ly, lbl, {
          size: FONT.small + 1,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
  });

  const defs = markerDefs(["arrow-filled"]);
  const body = el(
    "g",
    { transform: `translate(${n(0)},${n(0)})` },
    [...back, ...edgeSvg, ...front, ...labelSvg].join(""),
  );
  return svgDoc(maxX, maxY, body, defs);
}
