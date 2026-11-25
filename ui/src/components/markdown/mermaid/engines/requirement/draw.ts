/**
 * Requirement diagram drawing: SysML requirement and element cards laid out via layeredLayout,
 * semantic risk indicators, solid/dashed relationship connectors with markers, and relationship
 * stereotype pills.
 */
import { layeredLayout, roundedPath, type Point } from "../../layout/layered";
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import { el, line, markerDefs, markerRef, n, path, rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { RequirementKind, RequirementModel, RequirementNode, RiskLevel } from "./parse";

const PAD = 12;
const ROW_H = 17;
const SMALL = { size: FONT.small + 1 };
const BODY_BOLD = { size: FONT.body, bold: true };

const STEREOTYPES: Record<RequirementKind, string> = {
  requirement: "«Requirement»",
  functionalRequirement: "«Functional Requirement»",
  interfaceRequirement: "«Interface Requirement»",
  performanceRequirement: "«Performance Requirement»",
  physicalRequirement: "«Physical Requirement»",
  designConstraint: "«Design Constraint»",
};

const RISK_COLOR: Record<RiskLevel, string> = {
  Low: color.ok,
  Medium: color.warn,
  High: color.crit,
};

interface CardRow {
  lines: string[];
  accent?: string | undefined;
}

interface CardBox {
  width: number;
  height: number;
  headH: number;
  stereotype: string;
  title: string[];
  rows: CardRow[];
  isElement: boolean;
}

function cardOf(node: RequirementNode): CardBox {
  const isElement = node.kind === "element";
  const stereotype = isElement ? "«Element»" : STEREOTYPES[node.reqType];
  const title = wrapText(clampLabel(node.id), 240, BODY_BOLD, 2);

  const rows: CardRow[] = [];
  if (node.kind === "requirement") {
    if (node.reqId) rows.push({ lines: wrapText(`Id: ${node.reqId}`, 250, SMALL, 2) });
    if (node.text) rows.push({ lines: wrapText(`Text: ${node.text}`, 250, SMALL, 4) });
    if (node.risk) rows.push({ lines: [`Risk: ${node.risk}`], accent: RISK_COLOR[node.risk] });
    if (node.verifymethod) rows.push({ lines: [`Method: ${node.verifymethod}`] });
  } else {
    if (node.elemType) rows.push({ lines: wrapText(`Type: ${node.elemType}`, 250, SMALL, 2) });
    if (node.docRef) rows.push({ lines: wrapText(`DocRef: ${node.docRef}`, 250, SMALL, 2) });
  }

  let maxW = Math.max(
    textWidth(stereotype, { size: FONT.small }),
    measureLines(title, BODY_BOLD).width,
  );
  for (const row of rows) {
    maxW = Math.max(maxW, measureLines(row.lines, SMALL).width + (row.accent ? 12 : 0));
  }
  const width = Math.min(Math.max(maxW + PAD * 2, 160), 320);
  const headH = 14 + title.length * 18 + 10;
  const lineCount = rows.reduce((sum, r) => sum + r.lines.length, 0);
  const bodyH = rows.length > 0 ? lineCount * ROW_H + 12 : 8;
  return {
    width,
    height: headH + bodyH,
    headH,
    stereotype,
    title,
    rows,
    isElement,
  };
}

function drawCard(card: CardBox, cx: number, cy: number): string {
  const x = cx - card.width / 2;
  const top = cy - card.height / 2;
  const strip = card.isElement ? series(1) : series(0);

  let out = rect(x, top, card.width, card.height, {
    rx: 5,
    fill: color.nodeFill,
    stroke: color.nodeStroke,
    "stroke-width": 1.2,
  });
  out += rect(x, top, card.width, card.headH, {
    rx: 5,
    fill: color.groupHdr,
    stroke: "none",
  });
  out += rect(x, top + card.headH - 6, card.width, 6, { fill: color.groupHdr });
  out += rect(x, top, 4, card.height, { rx: 2, fill: strip, "fill-opacity": 0.85 });
  out += rect(x, top, card.width, card.height, {
    rx: 5,
    fill: "none",
    stroke: color.nodeStroke,
    "stroke-width": 1.2,
  });

  out += text(cx, top + 11, card.stereotype, {
    size: FONT.small,
    anchor: "middle",
    fill: color.textSec,
  });
  out += textLines(
    cx,
    top + 26,
    card.title,
    { size: FONT.body, weight: "600", anchor: "middle" },
    18 / FONT.body,
  );

  if (card.rows.length > 0) {
    out += line(x, top + card.headH, x + card.width, top + card.headH, {
      stroke: color.innerStroke,
      "stroke-width": 1,
    });
    let y = top + card.headH + 13;
    for (const row of card.rows) {
      if (row.accent) {
        out += rect(x + PAD, y - 4, 7, 7, { rx: 3.5, fill: row.accent });
      }
      const tx = x + PAD + (row.accent ? 12 : 0);
      const maxRowW = card.width - PAD * 2 - (row.accent ? 12 : 0);
      const fitted = row.lines.map((l) => fitWidth(l, maxRowW, SMALL));
      out += textLines(
        tx,
        y,
        fitted,
        { size: SMALL.size, fill: row.accent ? color.text : color.textSec },
        ROW_H / SMALL.size,
      );
      y += fitted.length * ROW_H;
    }
  }
  return out;
}

/** Draw a parsed requirement diagram as a complete passive SVG string. */
export function drawRequirement(model: RequirementModel): string {
  const budget = new StepBudget();
  const cards = new Map(model.nodes.map((node) => [node.id, cardOf(node)]));
  const layoutNodes = model.nodes.map((node) => ({
    id: node.id,
    width: cards.get(node.id)?.width ?? 160,
    height: cards.get(node.id)?.height ?? 60,
  }));
  const layoutEdges = model.relations.map((rel) => {
    const tag = `«${rel.kind}»`;
    return {
      from: rel.from,
      to: rel.to,
      labelWidth: textWidth(tag, SMALL) + 14,
      labelHeight: 20,
    };
  });

  const layout = layeredLayout(
    layoutNodes,
    layoutEdges,
    { direction: model.direction, rankGap: 72, nodeGap: 40 },
    budget,
  );

  let maxX = layout.width;
  const edgesSvg: string[] = [];
  const labelsSvg: string[] = [];

  model.relations.forEach((rel, i) => {
    const placed = layout.edges[i];
    if (!placed) return;
    const tag = `«${rel.kind}»`;
    let pts: Point[] = placed.points;
    if (placed.selfLoop) {
      const p = layout.nodes.get(rel.from);
      if (!p) return;
      const x = p.x + p.width / 2;
      pts = [
        [x, p.y - 12],
        [x + 32, p.y - 12],
        [x + 32, p.y + 12],
        [x, p.y + 12],
      ];
      const w = textWidth(tag, SMALL) + 14;
      placed.label = { x: x + 40 + w / 2, y: p.y, width: w, height: 20 };
      maxX = Math.max(maxX, x + 52 + w);
    }

    const isContainment = rel.kind === "contains";
    edgesSvg.push(
      path(roundedPath(pts, placed.selfLoop ? 6 : 8), {
        fill: "none",
        stroke: color.line,
        "stroke-width": 1.4,
        "stroke-dasharray": isContainment ? undefined : "6 4",
        "marker-end": markerRef(isContainment ? "arrow-filled" : "arrow-open"),
      }),
    );

    if (placed.label) {
      const w = textWidth(tag, SMALL) + 14;
      labelsSvg.push(
        rect(placed.label.x - w / 2, placed.label.y - 10, w, 20, {
          rx: 10,
          fill: color.canvas,
          stroke: color.innerStroke,
          "stroke-width": 0.8,
        }),
      );
      labelsSvg.push(
        text(placed.label.x, placed.label.y, tag, {
          size: SMALL.size,
          anchor: "middle",
          fill: color.textSec,
        }),
      );
    }
  });

  const nodesSvg = model.nodes.map((node) => {
    const p = layout.nodes.get(node.id);
    const card = cards.get(node.id);
    return p && card ? drawCard(card, p.x, p.y) : "";
  });

  const defs = markerDefs(["arrow-filled", "arrow-open"]);
  const body = el(
    "g",
    { transform: `translate(${n(0)},${n(0)})` },
    [...edgesSvg, ...nodesSvg, ...labelsSvg].join(""),
  );
  return svgDoc(maxX, layout.height, body, defs);
}
