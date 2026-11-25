/**
 * Ishikawa (fishbone) diagram drawing. Places top-level categories alternating above and below a
 * central horizontal spine pointing right into the effect head, with angled category bones and
 * left-extending horizontal sub-causes sized bottom-up so no branches overlap.
 */
import { StepBudget } from "../../limits";
import { measureLines, wrapText } from "../../measure";
import {
  g,
  line,
  markerDefs,
  markerRef,
  n,
  path,
  polygon,
  rect,
  svgDoc,
  textLines,
} from "../../svg";
import { FONT, color, series } from "../../theme";
import type { IshikawaCause, IshikawaModel } from "./parse";

const PAD = 24;
const SLANT = 0.48;
const TWIG = 18;
const SUB_STEP = 14;
const ROW_GAP = 8;
const CAT_GAP = 26;
const HEAD_GAP = 30;
const SPINE_PAD = 18;
const LINE_H = 16;
const SUB_MAX_W = 190;
const CAT_MAX_W = 160;
const HEAD_MAX_W = 180;

interface SubLayout {
  cause: IshikawaCause;
  lines: string[];
  labelW: number;
  rowH: number;
  subH: number;
  /** Leftmost offset relative to the parent attachment x (negative). */
  leftExtent: number;
  children: SubLayout[];
}

interface CategoryLayout {
  cause: IshikawaCause;
  index: number;
  side: -1 | 1;
  catLines: string[];
  catW: number;
  catH: number;
  boneH: number;
  /** Leftmost x relative to spine attachment (<= 0). */
  relLeft: number;
  /** Rightmost x relative to spine attachment (>= 0). */
  relRight: number;
  children: SubLayout[];
  spineX: number;
}

function measureSub(cause: IshikawaCause, depth: number, budget: StepBudget): SubLayout {
  budget.tick();
  const style = { size: FONT.small + 1 };
  const lines = wrapText(cause.label, SUB_MAX_W, style, 3);
  const measured = measureLines(lines, style, LINE_H / style.size);
  const rowH = Math.max(22, measured.height + 6);
  const children = cause.children.map((child) => measureSub(child, depth + 1, budget));
  const childrenH =
    children.length === 0
      ? 0
      : children.reduce((sum, c) => sum + c.subH, 0) + ROW_GAP * (children.length - 1);
  const subH = children.length === 0 ? rowH : rowH + ROW_GAP + childrenH;
  const ownLeft = (depth === 2 ? TWIG : SUB_STEP) + 8 + measured.width;
  const childLeft = children.reduce(
    (mx, c) => Math.max(mx, (depth === 2 ? TWIG : SUB_STEP) + c.leftExtent),
    0,
  );
  return {
    cause,
    lines,
    labelW: measured.width,
    rowH,
    subH,
    leftExtent: Math.max(ownLeft, childLeft),
    children,
  };
}

function measureCategory(
  cause: IshikawaCause,
  index: number,
  side: -1 | 1,
  budget: StepBudget,
): CategoryLayout {
  budget.tick();
  const headStyle = { size: FONT.body, bold: true };
  const catLines = wrapText(cause.label, CAT_MAX_W, headStyle, 3);
  const catSize = measureLines(catLines, headStyle, 18 / FONT.body);
  const catW = Math.max(76, catSize.width + 24);
  const catH = Math.max(30, catSize.height + 14);
  const children = cause.children.map((c) => measureSub(c, 2, budget));
  const bodyH =
    children.length === 0
      ? 36
      : children.reduce((sum, c) => sum + c.subH, 0) + ROW_GAP * (children.length - 1) + 28;
  const boneH = Math.max(58, bodyH + SPINE_PAD);
  const tipRelX = -boneH * SLANT;
  let relLeft = tipRelX - catW / 2;
  const relRight = Math.max(0, tipRelX + catW / 2);

  let yOffset = SPINE_PAD + 14;
  for (const child of children) {
    const rowDy = yOffset + child.rowH / 2;
    const boneX = -rowDy * SLANT;
    relLeft = Math.min(relLeft, boneX - child.leftExtent);
    yOffset += child.subH + ROW_GAP;
  }
  return {
    cause,
    index,
    side,
    catLines,
    catW,
    catH,
    boneH,
    relLeft,
    relRight,
    children,
    spineX: 0,
  };
}

function drawSubtree(
  node: SubLayout,
  attachX: number,
  topY: number,
  depth: number,
  accent: string,
  budget: StepBudget,
): string {
  budget.tick();
  const step = depth === 2 ? TWIG : SUB_STEP;
  const cy = topY + node.rowH / 2;
  const endX = attachX - step;
  let out = line(attachX, cy, endX, cy, { stroke: accent, "stroke-width": 1.2 });
  const textStartY = cy - ((node.lines.length - 1) * LINE_H) / 2;
  out += textLines(
    endX - 6,
    textStartY,
    node.lines,
    { size: FONT.small + 1, anchor: "end", fill: color.text },
    LINE_H / (FONT.small + 1),
  );
  if (node.children.length > 0) {
    const trunkX = endX - 6;
    let childTop = topY + node.rowH + ROW_GAP;
    let lastCy = cy;
    for (const child of node.children) {
      const childCy = childTop + child.rowH / 2;
      lastCy = childCy;
      out += drawSubtree(child, trunkX, childTop, depth + 1, accent, budget);
      childTop += child.subH + ROW_GAP;
    }
    out += line(trunkX, cy, trunkX, lastCy, {
      stroke: color.textFaint,
      "stroke-width": 1,
    });
  }
  return out;
}

function drawCategory(cat: CategoryLayout, spineY: number, budget: StepBudget): string {
  budget.tick();
  const accent = series(cat.index);
  const tipX = cat.spineX - cat.boneH * SLANT;
  const tipY = spineY + cat.side * cat.boneH;
  let out = line(cat.spineX, spineY, tipX, tipY, {
    stroke: accent,
    "stroke-width": 2,
    "stroke-linecap": "round",
  });

  const boxX = tipX - cat.catW / 2;
  const boxY = cat.side === -1 ? tipY - cat.catH : tipY;
  out += rect(boxX, boxY, cat.catW, cat.catH, {
    rx: 8,
    fill: accent,
    "fill-opacity": 0.16,
    stroke: accent,
    "stroke-width": 1.5,
  });
  const textStartY = boxY + cat.catH / 2 - ((cat.catLines.length - 1) * 18) / 2;
  out += textLines(
    tipX,
    textStartY,
    cat.catLines,
    { size: FONT.body, weight: "600", anchor: "middle", fill: color.text },
    18 / FONT.body,
  );

  if (cat.children.length > 0) {
    const totalBody =
      cat.children.reduce((s, c) => s + c.subH, 0) + ROW_GAP * (cat.children.length - 1);
    let cursorY = cat.side === -1 ? spineY - SPINE_PAD - totalBody : spineY + SPINE_PAD;
    for (const child of cat.children) {
      const childCy = cursorY + child.rowH / 2;
      const dy = Math.abs(childCy - spineY);
      const boneX = cat.spineX - dy * SLANT;
      out += drawSubtree(child, boneX, cursorY, 2, accent, budget);
      cursorY += child.subH + ROW_GAP;
    }
  }
  return g({}, out);
}

/** Draw a parsed Ishikawa (fishbone) diagram as a complete passive SVG string. */
export function drawIshikawa(model: IshikawaModel): string {
  const budget = new StepBudget();
  const cats = model.categories.map((c, idx) =>
    measureCategory(c, idx, idx % 2 === 0 ? -1 : 1, budget),
  );

  let topRight = PAD + 16;
  let botRight = PAD + 16;
  let lastSpineX = PAD + 16;

  for (const cat of cats) {
    budget.tick();
    const prevRight = cat.side === -1 ? topRight : botRight;
    const minSpine = Math.max(lastSpineX + 28, prevRight + CAT_GAP - cat.relLeft);
    cat.spineX = minSpine;
    lastSpineX = minSpine;
    const catMaxRight = minSpine + cat.relRight;
    if (cat.side === -1) topRight = catMaxRight;
    else botRight = catMaxRight;
  }

  const headStyle = { size: FONT.title, bold: true };
  const headLines = wrapText(model.effect, HEAD_MAX_W, headStyle, 4);
  const headSize = measureLines(headLines, headStyle, 22 / FONT.title);
  const headW = Math.max(104, headSize.width + 36);
  const headH = Math.max(54, headSize.height + 24);

  const topExtent = Math.max(
    headH / 2 + 12,
    ...cats.filter((c) => c.side === -1).map((c) => c.boneH + c.catH),
  );
  const botExtent = Math.max(
    headH / 2 + 12,
    ...cats.filter((c) => c.side === 1).map((c) => c.boneH + c.catH),
  );
  const spineY = PAD + topExtent;
  const tailX = PAD + 16;
  const headX = Math.max(topRight, botRight, lastSpineX) + HEAD_GAP;
  const width = headX + headW + PAD;
  const height = spineY + botExtent + PAD;

  let body = "";
  body += polygon(
    [
      [tailX - 14, spineY - 14],
      [tailX, spineY],
      [tailX - 14, spineY + 14],
    ],
    { fill: color.groupHdr, stroke: color.line, "stroke-width": 1.4, "stroke-linejoin": "round" },
  );
  body += line(tailX, spineY, headX - 4, spineY, {
    stroke: color.arrow,
    "stroke-width": 2.2,
    "marker-end": markerRef("arrow-filled"),
  });

  for (const cat of cats) {
    body += drawCategory(cat, spineY, budget);
  }

  const headTop = spineY - headH / 2;
  const headD = [
    `M${n(headX)},${n(headTop)}`,
    `L${n(headX + headW - 18)},${n(headTop)}`,
    `Q${n(headX + headW + 4)},${n(spineY)} ${n(headX + headW - 18)},${n(headTop + headH)}`,
    `L${n(headX)},${n(headTop + headH)}`,
    "Z",
  ].join(" ");
  body += path(headD, {
    fill: color.groupHdr,
    stroke: color.accent,
    "stroke-width": 2,
    "stroke-linejoin": "round",
  });
  const headTextStartY = spineY - ((headLines.length - 1) * 22) / 2;
  body += textLines(
    headX + (headW - 8) / 2,
    headTextStartY,
    headLines,
    { size: FONT.title, weight: "700", anchor: "middle", fill: color.text },
    22 / FONT.title,
  );

  return svgDoc(width, height, body, markerDefs(["arrow-filled"]));
}
