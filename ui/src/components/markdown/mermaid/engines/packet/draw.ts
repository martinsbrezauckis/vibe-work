/**
 * Packet diagram drawing: 32-bit rows of rectangular bit-range cells tinted with category tokens,
 * bit-position labels above cell edges, wrapped/fitted labels inside cells, and an optional title.
 */
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, textWidth, wrapText } from "../../measure";
import { rect, svgDoc, text, textLines } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { PacketModel } from "./parse";

const BIT_W = 32;
const BIT_HDR_H = 16;
const CELL_H = 44;
const ROW_GAP = 14;
const PAD = 20;

/** Draw a parsed packet diagram as a complete passive SVG string. */
export function drawPacket(model: PacketModel): string {
  const budget = new StepBudget();
  const cols = model.bitsPerRow;
  const maxBit = Math.max(...model.fields.map((f) => f.end));
  const rowCount = Math.floor(maxBit / cols) + 1;

  const titleText = model.title
    ? fitWidth(clampLabel(model.title), cols * BIT_W, { size: FONT.title, bold: true })
    : "";
  const titleH = titleText ? 34 : 0;
  const rowPitch = BIT_HDR_H + CELL_H + ROW_GAP;
  const width = Math.max(
    PAD * 2 + cols * BIT_W,
    titleText ? textWidth(titleText, { size: FONT.title, bold: true }) + PAD * 2 : 0,
  );
  const height = PAD * 2 + titleH + rowCount * rowPitch - ROW_GAP;

  let body = "";
  if (titleText) {
    body += text(width / 2, PAD + 10, titleText, {
      size: FONT.title,
      weight: "600",
      anchor: "middle",
    });
  }

  const gridTop = PAD + titleH;
  const gridLeft = (width - cols * BIT_W) / 2;

  model.fields.forEach((field, fieldIndex) => {
    const startRow = Math.floor(field.start / cols);
    const endRow = Math.floor(field.end / cols);
    const fill = series(fieldIndex);
    const label = clampLabel(field.label);

    for (let row = startRow; row <= endRow; row++) {
      budget.tick();
      const segStart = Math.max(field.start, row * cols);
      const segEnd = Math.min(field.end, (row + 1) * cols - 1);
      const colStart = segStart % cols;
      const span = segEnd - segStart + 1;
      const x = gridLeft + colStart * BIT_W;
      const w = span * BIT_W;
      const y = gridTop + row * rowPitch + BIT_HDR_H;

      if (span === 1) {
        body += text(x + w / 2, y - 7, String(segStart), {
          size: FONT.small - 1,
          anchor: "middle",
          fill: color.textMuted,
        });
      } else {
        body += text(x + 4, y - 7, String(segStart), {
          size: FONT.small - 1,
          anchor: "start",
          fill: color.textMuted,
        });
        body += text(x + w - 4, y - 7, String(segEnd), {
          size: FONT.small - 1,
          anchor: "end",
          fill: color.textMuted,
        });
      }

      body += rect(x, y, w, CELL_H, {
        rx: 3,
        fill,
        "fill-opacity": 0.16,
      });
      body += rect(x, y, w, CELL_H, {
        rx: 3,
        fill: "none",
        stroke: color.nodeStroke,
        "stroke-width": 1.2,
      });

      if (w >= 84) {
        const style = { size: FONT.small + 1, bold: true };
        const lines = wrapText(label, w - 12, style, 2);
        const lineHeight = 1.25;
        const step = style.size * lineHeight;
        const firstY = y + CELL_H / 2 - ((lines.length - 1) * step) / 2;
        body += textLines(
          x + w / 2,
          firstY,
          lines,
          { size: style.size, weight: "500", anchor: "middle" },
          lineHeight,
        );
      } else {
        const fitted = fitWidth(label, w - 6, { size: FONT.small });
        body += text(x + w / 2, y + CELL_H / 2, fitted, {
          size: FONT.small,
          weight: "500",
          anchor: "middle",
        });
      }
    }
  });

  return svgDoc(width, height, body);
}
