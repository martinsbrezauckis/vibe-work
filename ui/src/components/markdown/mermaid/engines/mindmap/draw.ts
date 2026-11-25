/**
 * Mindmap diagram drawing. Lays out top-level branches symmetrically left and right around the
 * centered root, draws smooth cubic Bezier branch connectors colored by top-level branch, and
 * renders all seven documented node shapes plus rich markdown bold/italic tspans.
 */
import { StepBudget } from "../../limits";
import { clampLabel, textWidth, wrapText } from "../../measure";
import { circle, el, esc, g, n, path, polygon, rect, svgDoc, text } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { MindmapModel, MindmapNode } from "./parse";

const PAD = 24;
const H_GAP = 42;
const V_GAP = 14;
const MAX_TEXT_W = 200;
const LINE_STEP = 18;

interface Run {
  text: string;
  bold: boolean;
  italic: boolean;
}

interface LaidNode {
  node: MindmapNode;
  lines: Run[][];
  textW: number;
  textH: number;
  w: number;
  h: number;
  subH: number;
  x: number;
  y: number;
  children: LaidNode[];
}

function parseMarkdownRuns(paragraph: string): Run[] {
  const runs: Run[] = [];
  const pattern = /(\*\*([^*]+)\*\*|\*([^*]+)\*)/g;
  let last = 0;
  for (const match of paragraph.matchAll(pattern)) {
    const idx = match.index;
    if (idx > last) {
      runs.push({ text: paragraph.slice(last, idx), bold: false, italic: false });
    }
    if (match[2] !== undefined) {
      runs.push({ text: match[2], bold: true, italic: false });
    } else if (match[3] !== undefined) {
      runs.push({ text: match[3], bold: false, italic: true });
    }
    last = idx + match[0].length;
  }
  if (last < paragraph.length) {
    runs.push({ text: paragraph.slice(last), bold: false, italic: false });
  }
  return runs.length > 0 ? runs : [{ text: "", bold: false, italic: false }];
}

function wrapMarkdown(raw: string, size: number): Run[][] {
  const out: Run[][] = [];
  for (const para of clampLabel(raw).split(/<br\s*\/?>|\n/i)) {
    const runs = parseMarkdownRuns(para);
    const words: { word: string; bold: boolean; italic: boolean }[] = [];
    for (const run of runs) {
      for (const part of run.text.split(/(\s+)/)) {
        if (!part || /^\s+$/.test(part)) continue;
        words.push({ word: part, bold: run.bold, italic: run.italic });
      }
    }
    if (words.length === 0) {
      out.push([{ text: "", bold: false, italic: false }]);
      continue;
    }
    let line: Run[] = [];
    let lineW = 0;
    const spaceW = textWidth(" ", { size });
    for (const tok of words) {
      const w = textWidth(tok.word, { size, bold: tok.bold });
      if (line.length > 0 && lineW + spaceW + w > MAX_TEXT_W) {
        out.push(line);
        line = [{ text: tok.word, bold: tok.bold, italic: tok.italic }];
        lineW = w;
      } else {
        const last = line[line.length - 1];
        if (last && last.bold === tok.bold && last.italic === tok.italic) {
          last.text += ` ${tok.word}`;
        } else {
          if (line.length > 0 && last) last.text += " ";
          line.push({ text: tok.word, bold: tok.bold, italic: tok.italic });
        }
        lineW += (line.length > 1 || (last && last.text.includes(" ")) ? spaceW : 0) + w;
      }
    }
    if (line.length > 0) out.push(line);
  }
  return out.slice(0, 8);
}

function measureNode(node: MindmapNode, budget: StepBudget): LaidNode {
  budget.tick();
  const isRoot = node.depth === 0;
  const size = isRoot ? FONT.body + 1 : FONT.body;
  const lines: Run[][] = node.isMarkdown
    ? wrapMarkdown(node.label, size)
    : wrapText(node.label, MAX_TEXT_W, { size, bold: isRoot }, 6).map((l) => [
        { text: l, bold: isRoot, italic: false },
      ]);

  let textW = 0;
  for (const row of lines) {
    let rowW = 0;
    for (const run of row) rowW += textWidth(run.text, { size, bold: run.bold });
    textW = Math.max(textW, rowW);
  }
  const textH = Math.max(LINE_STEP, lines.length * LINE_STEP);

  let w = textW + 26;
  let h = textH + 14;
  if (node.shape === "circle") {
    if (textW <= 110) {
      const diameter = Math.max(54, Math.hypot(textW, textH) + 22);
      w = diameter;
      h = diameter;
    } else {
      w = textW + 38;
      h = textH + 28;
    }
  } else if (node.shape === "hexagon") {
    w = textW + 44;
    h = Math.max(36, textH + 18);
  } else if (node.shape === "cloud" || node.shape === "bang") {
    w = textW + 46;
    h = Math.max(42, textH + 26);
  } else if (node.shape === "rounded") {
    w = textW + 28;
    h = textH + 16;
  }

  const children = node.children.map((child) => measureNode(child, budget));
  const childrenH =
    children.length === 0
      ? 0
      : children.reduce((s, c) => s + c.subH, 0) + V_GAP * (children.length - 1);
  return {
    node,
    lines,
    textW,
    textH,
    w,
    h,
    subH: Math.max(h, childrenH),
    x: 0,
    y: 0,
    children,
  };
}

function placeSide(
  branches: LaidNode[],
  parentEdgeX: number,
  topY: number,
  totalH: number,
  dir: 1 | -1,
  budget: StepBudget,
): void {
  const childrenH =
    branches.reduce((s, b) => s + b.subH, 0) + V_GAP * Math.max(0, branches.length - 1);
  let cursorY = topY + (totalH - childrenH) / 2;
  for (const child of branches) {
    budget.tick();
    const cy = cursorY + child.subH / 2;
    child.x = parentEdgeX + dir * (H_GAP + child.w / 2);
    child.y = cy;
    if (child.children.length > 0) {
      placeSide(child.children, child.x + dir * (child.w / 2), cursorY, child.subH, dir, budget);
    }
    cursorY += child.subH + V_GAP;
  }
}

function cloudPath(cx: number, cy: number, w: number, h: number): string {
  const x0 = cx - w / 2 + 8;
  const x1 = cx + w / 2 - 8;
  const y0 = cy - h / 2 + 6;
  const y1 = cy + h / 2 - 6;
  const mx = cx;
  const my = cy;
  return [
    `M${n(x0)},${n(my)}`,
    `Q${n(x0 - 12)},${n(y0)} ${n((x0 + mx) / 2)},${n(y0)}`,
    `Q${n(mx)},${n(y0 - 12)} ${n((mx + x1) / 2)},${n(y0)}`,
    `Q${n(x1 + 12)},${n(y0)} ${n(x1)},${n(my)}`,
    `Q${n(x1 + 12)},${n(y1)} ${n((mx + x1) / 2)},${n(y1)}`,
    `Q${n(mx)},${n(y1 + 12)} ${n((x0 + mx) / 2)},${n(y1)}`,
    `Q${n(x0 - 12)},${n(y1)} ${n(x0)},${n(my)}`,
    "Z",
  ].join(" ");
}

function bangPath(cx: number, cy: number, w: number, h: number): string {
  const spikes = 12;
  const rx = w / 2;
  const ry = h / 2;
  const parts: string[] = [];
  for (let i = 0; i < spikes; i++) {
    const a1 = (i / spikes) * Math.PI * 2;
    const a2 = ((i + 0.5) / spikes) * Math.PI * 2;
    const xOut = cx + rx * Math.cos(a1);
    const yOut = cy + ry * Math.sin(a1);
    const xIn = cx + rx * 0.78 * Math.cos(a2);
    const yIn = cy + ry * 0.78 * Math.sin(a2);
    parts.push(`${i === 0 ? "M" : "L"}${n(xOut)},${n(yOut)} L${n(xIn)},${n(yIn)}`);
  }
  parts.push("Z");
  return parts.join(" ");
}

function drawRichLines(node: LaidNode, cx: number, cy: number): string {
  const size = node.node.depth === 0 ? FONT.body + 1 : FONT.body;
  const startY = cy - ((node.lines.length - 1) * LINE_STEP) / 2;
  return node.lines
    .map((runs, rowIdx) => {
      const y = startY + rowIdx * LINE_STEP;
      const first = runs[0];
      if (runs.length === 1 && first) {
        return text(cx, y, first.text, {
          size,
          weight: first.bold ? "700" : "400",
          italic: first.italic,
          anchor: "middle",
          fill: color.text,
        });
      }
      const tspans = runs
        .map((run) =>
          el(
            "tspan",
            {
              "font-weight": run.bold ? "700" : "400",
              "font-style": run.italic ? "italic" : undefined,
            },
            esc(run.text),
          ),
        )
        .join("");
      return el(
        "text",
        {
          x: cx,
          y,
          dy: "0.35em",
          "text-anchor": "middle",
          "font-size": size,
          fill: color.text,
        },
        tspans,
      );
    })
    .join("");
}

function drawShape(laid: LaidNode): string {
  const { node, x: cx, y: cy, w, h } = laid;
  const isRoot = node.depth === 0;
  const accent = isRoot ? color.accent : series(node.branch);
  const fill = isRoot ? color.groupHdr : accent;
  const fillOpacity = isRoot ? 1 : node.depth === 1 ? 0.22 : 0.12;
  const strokeWidth = isRoot ? 2 : node.depth === 1 ? 1.7 : 1.2;
  const styleAttrs = {
    fill,
    "fill-opacity": fillOpacity,
    stroke: accent,
    "stroke-width": strokeWidth,
  };

  let outline: string;
  switch (node.shape) {
    case "circle":
      outline =
        Math.abs(w - h) < 1
          ? circle(cx, cy, w / 2, styleAttrs)
          : el("ellipse", { cx, cy, rx: w / 2, ry: h / 2, ...styleAttrs });
      break;
    case "square":
      outline = rect(cx - w / 2, cy - h / 2, w, h, { rx: 2, ...styleAttrs });
      break;
    case "rounded":
      outline = rect(cx - w / 2, cy - h / 2, w, h, { rx: 12, ...styleAttrs });
      break;
    case "hexagon": {
      const bevel = 12;
      const l = cx - w / 2;
      const r = cx + w / 2;
      const t = cy - h / 2;
      const b = cy + h / 2;
      outline = polygon(
        [
          [l + bevel, t],
          [r - bevel, t],
          [r, cy],
          [r - bevel, b],
          [l + bevel, b],
          [l, cy],
        ],
        styleAttrs,
      );
      break;
    }
    case "cloud":
      outline = path(cloudPath(cx, cy, w, h), styleAttrs);
      break;
    case "bang":
      outline = path(bangPath(cx, cy, w, h), styleAttrs);
      break;
    default:
      outline = rect(cx - w / 2, cy - h / 2, w, h, { rx: 6, ...styleAttrs });
      break;
  }
  return g({}, outline + drawRichLines(laid, cx, cy));
}

/** Draw a parsed mindmap diagram as a complete passive SVG string. */
export function drawMindmap(model: MindmapModel): string {
  const budget = new StepBudget();
  const root = measureNode(model.root, budget);

  const rightBranches: LaidNode[] = [];
  const leftBranches: LaidNode[] = [];
  let rightH = 0;
  let leftH = 0;
  for (const branch of root.children) {
    if (rightBranches.length === 0 || rightH <= leftH) {
      rightBranches.push(branch);
      rightH += branch.subH + (rightBranches.length > 1 ? V_GAP : 0);
    } else {
      leftBranches.push(branch);
      leftH += branch.subH + (leftBranches.length > 1 ? V_GAP : 0);
    }
  }

  const totalH = Math.max(root.h, rightH, leftH);
  root.x = 0;
  root.y = totalH / 2;
  if (rightBranches.length > 0) {
    placeSide(rightBranches, root.x + root.w / 2, 0, totalH, 1, budget);
  }
  if (leftBranches.length > 0) {
    placeSide(leftBranches, root.x - root.w / 2, 0, totalH, -1, budget);
  }

  const all: LaidNode[] = [];
  const edges: { parent: LaidNode; child: LaidNode; dir: 1 | -1 }[] = [];
  const collect = (node: LaidNode, dir: 1 | -1): void => {
    budget.tick();
    all.push(node);
    for (const child of node.children) {
      edges.push({ parent: node, child, dir });
      collect(child, dir);
    }
  };
  all.push(root);
  for (const branch of rightBranches) {
    edges.push({ parent: root, child: branch, dir: 1 });
    collect(branch, 1);
  }
  for (const branch of leftBranches) {
    edges.push({ parent: root, child: branch, dir: -1 });
    collect(branch, -1);
  }

  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  for (const node of all) {
    minX = Math.min(minX, node.x - node.w / 2);
    minY = Math.min(minY, node.y - node.h / 2);
    maxX = Math.max(maxX, node.x + node.w / 2);
    maxY = Math.max(maxY, node.y + node.h / 2);
  }

  const shiftX = PAD - minX;
  const shiftY = PAD - minY;
  for (const node of all) {
    node.x += shiftX;
    node.y += shiftY;
  }

  const edgeMarkup = edges
    .map(({ parent, child, dir }) => {
      const x1 = parent.x + dir * (parent.w / 2);
      const y1 = parent.y;
      const x2 = child.x - dir * (child.w / 2);
      const y2 = child.y;
      const mx = (x1 + x2) / 2;
      const sw = child.node.depth === 1 ? 2.4 : child.node.depth === 2 ? 1.8 : 1.3;
      return path(`M${n(x1)},${n(y1)} C${n(mx)},${n(y1)} ${n(mx)},${n(y2)} ${n(x2)},${n(y2)}`, {
        fill: "none",
        stroke: series(child.node.branch),
        "stroke-width": sw,
        "stroke-linecap": "round",
      });
    })
    .join("");

  const nodeMarkup = all.map(drawShape).join("");
  const width = maxX - minX + PAD * 2;
  const height = maxY - minY + PAD * 2;
  return svgDoc(width, height, g({}, edgeMarkup) + g({}, nodeMarkup));
}
