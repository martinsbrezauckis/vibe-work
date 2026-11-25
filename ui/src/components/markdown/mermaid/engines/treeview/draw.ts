/**
 * TreeView diagram drawing. Renders directory hierarchies as indented rows with elbow connector
 * lines, optional vector file/folder/unregistered icons, :::highlight pills, bold directory names,
 * and italic `##` inline descriptions using app theme tokens only.
 */
import { StepBudget } from "../../limits";
import { clampLabel, textWidth } from "../../measure";
import { g, path, rect, svgDoc, text } from "../../svg";
import { FONT, color } from "../../theme";
import type { TreeViewModel, TreeViewNode } from "./parse";

const PAD = 20;
const INDENT = 24;
const ROW_H = 26;
const ICON_SIZE = 14;
const ICON_GAP = 6;
const DESC_GAP = 12;

interface PlacedRow {
  node: TreeViewNode;
  y: number;
  x: number;
  childRows: number[];
}

function drawIcon(kind: string, x: number, cy: number): string {
  const top = cy - ICON_SIZE / 2;
  if (kind === "folder") {
    const d = `M${String(x)},${String(top + 2)} L${String(x + 5)},${String(top + 2)} L${String(x + 7)},${String(top + 4)} L${String(x + 14)},${String(top + 4)} L${String(x + 14)},${String(top + 12)} L${String(x)},${String(top + 12)} Z`;
    return path(d, {
      fill: color.accent,
      "fill-opacity": 0.24,
      stroke: color.accent,
      "stroke-width": 1.2,
      "stroke-linejoin": "round",
    });
  }
  if (kind === "file") {
    const d = `M${String(x + 2)},${String(top + 1)} L${String(x + 9)},${String(top + 1)} L${String(x + 12)},${String(top + 4)} L${String(x + 12)},${String(top + 13)} L${String(x + 2)},${String(top + 13)} Z`;
    const fold = `M${String(x + 9)},${String(top + 1)} L${String(x + 9)},${String(top + 4)} L${String(x + 12)},${String(top + 4)}`;
    return (
      path(d, {
        fill: color.nodeFill,
        stroke: color.textSec,
        "stroke-width": 1.2,
        "stroke-linejoin": "round",
      }) + path(fold, { fill: "none", stroke: color.textSec, "stroke-width": 1 })
    );
  }
  return g(
    {},
    rect(x, top, ICON_SIZE, ICON_SIZE, {
      rx: 3,
      fill: color.groupHdr,
      stroke: color.nodeStroke,
      "stroke-width": 1,
    }) +
      text(x + ICON_SIZE / 2, cy, "?", {
        size: FONT.small - 1,
        weight: "600",
        anchor: "middle",
        fill: color.textMuted,
      }),
  );
}

/** Draw a parsed TreeView diagram as a complete passive SVG string. */
export function drawTreeview(model: TreeViewModel): string {
  const budget = new StepBudget();
  const rows: PlacedRow[] = [];

  const visit = (node: TreeViewNode): number => {
    budget.tick();
    const idx = rows.length;
    const row: PlacedRow = {
      node,
      y: PAD + idx * ROW_H + ROW_H / 2,
      x: PAD + node.depth * INDENT,
      childRows: [],
    };
    rows.push(row);
    for (const child of node.children) {
      row.childRows.push(visit(child));
    }
    return idx;
  };
  for (const root of model.roots) {
    visit(root);
  }

  let connectors = "";
  let content = "";
  let maxX = 120;

  for (const row of rows) {
    budget.tick();
    const lastChildIdx = row.childRows[row.childRows.length - 1];
    if (lastChildIdx !== undefined) {
      const trunkX = row.x + 7;
      const lastChild = rows[lastChildIdx];
      if (lastChild) {
        const startY = row.y + ROW_H / 2 - 3;
        connectors += path(
          `M${String(trunkX)},${String(startY)} L${String(trunkX)},${String(lastChild.y)}`,
          {
            fill: "none",
            stroke: color.line,
            "stroke-width": 1.2,
          },
        );
      }
      for (const childIdx of row.childRows) {
        const child = rows[childIdx];
        if (!child) continue;
        connectors += path(
          `M${String(trunkX)},${String(child.y)} L${String(child.x - 4)},${String(child.y)}`,
          {
            fill: "none",
            stroke: color.line,
            "stroke-width": 1.2,
          },
        );
      }
    }

    const icon = row.node.icon;
    const hasIcon = icon !== undefined && icon !== "none";
    const labelX = hasIcon ? row.x + ICON_SIZE + ICON_GAP : row.x;
    const labelText = clampLabel(row.node.label);
    const labelW = textWidth(labelText, { size: FONT.body, bold: row.node.isDir });
    const descText = row.node.description ? clampLabel(row.node.description) : undefined;
    const descW = descText ? DESC_GAP + textWidth(descText, { size: FONT.small + 1 }) : 0;
    const rowRight = labelX + labelW + descW;
    maxX = Math.max(maxX, rowRight);

    if (row.node.highlight) {
      const pillX = row.x - 5;
      const pillW = rowRight - pillX + 6;
      content += rect(pillX, row.y - 10, pillW, 20, {
        rx: 4,
        fill: color.warn,
        "fill-opacity": 0.16,
        stroke: color.warn,
        "stroke-width": 1,
      });
    }
    if (hasIcon) {
      content += drawIcon(icon, row.x, row.y);
    }
    content += text(labelX, row.y, labelText, {
      size: FONT.body,
      weight: row.node.isDir ? "700" : "400",
      fill: color.text,
    });
    if (descText) {
      content += text(labelX + labelW + DESC_GAP, row.y, descText, {
        size: FONT.small + 1,
        italic: true,
        fill: color.ok,
      });
    }
  }

  const width = maxX + PAD;
  const height = PAD * 2 + rows.length * ROW_H;
  return svgDoc(width, height, g({}, connectors) + g({}, content));
}
