/**
 * Sequence diagram drawing: header and footer participant boxes (actors as figures), dashed
 * lifelines, activation bars, arrows in ten styles, notes, nested block frames and autonumber badges.
 * Columns are sized from the text that has to fit between them; the drawing is then shifted so that
 * nothing leaves the viewBox.
 */
import { DiagramInputError } from "../../types";
import { StepBudget } from "../../limits";
import { fitWidth, measureLines, textWidth, wrapText } from "../../measure";
import {
  circle,
  el,
  g,
  line,
  markerDefs,
  markerRef,
  n,
  path,
  polyline,
  rect,
  svgDoc,
  text,
  textLines,
  type MarkerName,
} from "../../svg";
import { FONT, color } from "../../theme";
import type { ArrowKind, SequenceModel, SequenceParticipant, SequenceStatement } from "./parse";

const MARGIN = 24;
const MIN_GAP = 120;
const MSG_WRAP = 260;
const NOTE_WRAP = 220;
const LINE = FONT.body * 1.3;
const BAR = 10;
const SELF_W = 32;

interface Header {
  p: SequenceParticipant;
  width: number;
  height: number;
  lines: string[];
}

const ARROW_STYLE: Record<ArrowKind, { dashed: boolean; end?: MarkerName; start?: MarkerName }> = {
  solid: { dashed: false },
  dotted: { dashed: true },
  "solid-arrow": { dashed: false, end: "arrow-filled" },
  "dotted-arrow": { dashed: true, end: "arrow-filled" },
  "solid-cross": { dashed: false, end: "arrow-cross" },
  "dotted-cross": { dashed: true, end: "arrow-cross" },
  "solid-async": { dashed: false, end: "arrow-open" },
  "dotted-async": { dashed: true, end: "arrow-open" },
  "solid-both": { dashed: false, start: "arrow-filled", end: "arrow-filled" },
  "dotted-both": { dashed: true, start: "arrow-filled", end: "arrow-filled" },
};

function headerOf(p: SequenceParticipant): Header {
  const style = { size: FONT.body, bold: true };
  const lines = wrapText(p.label, 170, style, 3);
  const size = measureLines(lines, style, 1.3);
  if (p.kind === "actor") {
    return {
      p,
      lines,
      width: Math.max(56, size.width + 12),
      height: 52 + size.height,
    };
  }
  return {
    p,
    lines,
    width: Math.max(84, size.width + 28),
    height: Math.max(38, size.height + 18),
  };
}

function drawHeader(h: Header, cx: number, top: number): string {
  const style = { size: FONT.body, weight: "600" as const, anchor: "middle" as const };
  if (h.p.kind === "actor") {
    const stroke = { stroke: color.nodeStroke, "stroke-width": 1.5, fill: "none" };
    return (
      g(
        {},
        circle(cx, top + 9, 7, { ...stroke, fill: color.nodeFill }) +
          line(cx, top + 16, cx, top + 33, stroke) +
          line(cx - 10, top + 23, cx + 10, top + 23, stroke) +
          polyline(
            [
              [cx - 9, top + 46],
              [cx, top + 33],
              [cx + 9, top + 46],
            ],
            stroke,
          ),
      ) + textLines(cx, top + 58, h.lines, style)
    );
  }
  return (
    rect(cx - h.width / 2, top, h.width, h.height, {
      rx: 5,
      fill: color.nodeFill,
      stroke: color.nodeStroke,
      "stroke-width": 1,
    }) + textLines(cx, top + h.height / 2 - ((h.lines.length - 1) * LINE) / 2, h.lines, style)
  );
}

/** Draw a parsed sequence diagram as a complete SVG string. */
export function drawSequence(model: SequenceModel): string {
  const budget = new StepBudget();
  const headers = model.participants.map(headerOf);
  const index = new Map(model.participants.map((p, i) => [p.id, i]));
  const at = (id: string): number => {
    const i = index.get(id);
    if (i === undefined) throw new DiagramInputError("Unknown participant");
    return i;
  };
  const msgStyle = { size: FONT.body };
  const noteStyle = { size: FONT.small + 1 };

  // Column spacing from the text that has to fit.
  const count = headers.length;
  const gap = headers.map((h, i) => {
    const next = headers[i + 1];
    return next ? Math.max(MIN_GAP, h.width / 2 + next.width / 2 + 24) : 0;
  });
  let leftRoom = MARGIN + (headers[0]?.width ?? 0) / 2;
  let rightRoom = MARGIN + (headers[count - 1]?.width ?? 0) / 2;
  const spans: { from: number; to: number; need: number }[] = [];
  const needGap = (i: number, need: number): void => {
    gap[i] = Math.max(gap[i] ?? 0, need);
  };
  const visit = (statements: SequenceStatement[]): void => {
    for (const s of statements) {
      budget.tick();
      if (s.kind === "message") {
        const w = measureLines(wrapText(s.text, MSG_WRAP, msgStyle), msgStyle).width;
        const a = at(s.from);
        const b = at(s.to);
        if (a === b) {
          if (a === count - 1) rightRoom = Math.max(rightRoom, w + SELF_W + 24);
          else needGap(a, w + SELF_W + 24);
        } else spans.push({ from: Math.min(a, b), to: Math.max(a, b), need: w + 44 });
      } else if (s.kind === "note") {
        const w = measureLines(wrapText(s.text, NOTE_WRAP, noteStyle), noteStyle).width + 20;
        const a = at(s.participants[0] ?? "");
        if (s.placement === "left") {
          if (a === 0) leftRoom = Math.max(leftRoom, w + 24);
          else needGap(a - 1, w + 24);
        } else if (s.placement === "right") {
          if (a === count - 1) rightRoom = Math.max(rightRoom, w + 24);
          else needGap(a, w + 24);
        } else if (s.participants.length === 2) {
          const b = at(s.participants[1] ?? "");
          spans.push({ from: Math.min(a, b), to: Math.max(a, b), need: w - 20 });
        }
      } else if (s.kind === "block") {
        for (const section of s.sections) visit(section.statements);
      }
    }
  };
  visit(model.statements);
  spans.sort((x, y) => x.to - x.from - (y.to - y.from));
  for (const { from, to, need } of spans) {
    let have = 0;
    for (let i = from; i < to; i++) have += gap[i] ?? 0;
    if (have < need)
      for (let i = from; i < to; i++) gap[i] = (gap[i] ?? 0) + (need - have) / (to - from);
  }
  const centers: number[] = [];
  let cursor = leftRoom;
  headers.forEach((_, i) => {
    centers.push(cursor);
    cursor += gap[i] ?? 0;
  });
  const cx = (id: string): number => centers[at(id)] ?? 0;

  // Vertical walk.
  const titleLines = model.title
    ? wrapText(model.title, 520, { size: FONT.title, bold: true }, 2)
    : [];
  const titleHeight = titleLines.length ? titleLines.length * FONT.title * 1.3 + 14 : 0;
  const boxBand = model.boxes.length ? 30 : 0;
  const headerTop = MARGIN + titleHeight + boxBand;
  const headerHeight = Math.max(0, ...headers.map((h) => h.height));
  let y = headerTop + headerHeight + 22;
  let minX = 0;
  let maxX = 0;
  const touch = (x1: number, x2: number): void => {
    minX = Math.min(minX, x1);
    maxX = Math.max(maxX, x2);
  };
  centers.forEach((c, i) => {
    const w = headers[i]?.width ?? 0;
    touch(c - w / 2 - MARGIN, c + w / 2 + MARGIN);
  });
  touch(0, cursor - (gap[count - 1] ?? 0) + rightRoom);
  const back: string[] = [];
  const front: string[] = [];
  /** Block tabs and section labels: drawn above activation bars so their text stays readable. */
  const chrome: string[] = [];
  const bars: { x: number; y1: number; y2: number }[] = [];
  const open = new Map<string, number[]>();
  const created = new Map<string, number>();
  const ended = new Map<string, number>();
  let counter = model.autonumber?.start ?? 1;

  const depthOf = (id: string): number => open.get(id)?.length ?? 0;
  const barX = (id: string, level: number): number => cx(id) - BAR / 2 + level * (BAR / 2);
  const edgeX = (id: string, towards: number): number => {
    const d = depthOf(id);
    if (d === 0) return cx(id);
    return towards > 0 ? cx(id) + BAR / 2 + (d - 1) * (BAR / 2) : cx(id) - BAR / 2;
  };
  const begin = (id: string, at0: number): void => {
    open.set(id, [...(open.get(id) ?? []), at0]);
  };
  const finish = (id: string, at1: number): void => {
    const stack = open.get(id);
    const start = stack?.pop();
    if (start !== undefined && stack) {
      bars.push({ x: barX(id, stack.length), y1: start, y2: Math.max(at1, start + 6) });
    }
  };

  const extentOf = (statements: SequenceStatement[]): [number, number] | undefined => {
    let lo = Number.POSITIVE_INFINITY;
    let hi = Number.NEGATIVE_INFINITY;
    const add = (a: number, b: number): void => {
      lo = Math.min(lo, a);
      hi = Math.max(hi, b);
    };
    for (const s of statements) {
      if (s.kind === "message") {
        add(Math.min(cx(s.from), cx(s.to)), Math.max(cx(s.from), cx(s.to)));
        if (s.from === s.to) {
          const w = measureLines(wrapText(s.text, MSG_WRAP, msgStyle), msgStyle).width;
          add(cx(s.from), cx(s.from) + SELF_W + 12 + w);
        }
      } else if (s.kind === "note") {
        const a = cx(s.participants[0] ?? "");
        const w = measureLines(wrapText(s.text, NOTE_WRAP, noteStyle), noteStyle).width + 20;
        if (s.placement === "left") add(a - 12 - w, a);
        else if (s.placement === "right") add(a, a + 12 + w);
        else
          add(Math.min(...s.participants.map(cx)) - 10, Math.max(...s.participants.map(cx)) + 10);
      } else if (s.kind === "block") {
        for (const section of s.sections) {
          const inner = extentOf(section.statements);
          if (inner) add(inner[0] - 12, inner[1] + 12);
        }
      } else add(cx(s.id), cx(s.id));
    }
    return lo <= hi ? [lo, hi] : undefined;
  };

  const message = (s: Extract<SequenceStatement, { kind: "message" }>): void => {
    const style = ARROW_STYLE[s.arrow];
    const lines = wrapText(s.text, MSG_WRAP, msgStyle);
    const textH = lines.length * LINE;
    const from = at(s.from);
    const to = at(s.to);
    const self = from === to;
    if (s.created) {
      const h = headers[to];
      if (h) {
        created.set(s.to, y + h.height / 2 - 6);
        y += Math.max(0, h.height / 2 - 12);
      }
    }
    const dir = Math.sign((centers[to] ?? 0) - (centers[from] ?? 0)) || 1;
    const stroke = {
      stroke: color.line,
      "stroke-width": 1.4,
      "stroke-dasharray": style.dashed ? "6 4" : undefined,
      "marker-start": style.start ? markerRef(style.start) : undefined,
      "marker-end": style.end ? markerRef(style.end) : undefined,
      fill: "none",
    };
    let arrowY: number;
    let badgeX: number;
    if (self) {
      y += 8;
      const x0 = edgeX(s.from, 1);
      const rowH = Math.max(24, textH);
      arrowY = y + rowH / 2 - 12;
      front.push(
        path(
          `M${n(x0)},${n(arrowY)} L${n(x0 + SELF_W)},${n(arrowY)} L${n(x0 + SELF_W)},${n(arrowY + 22)} L${n(x0 + 1)},${n(arrowY + 22)}`,
          stroke,
        ),
      );
      front.push(
        textLines(
          x0 + SELF_W + 10,
          y + rowH / 2 - ((lines.length - 1) * LINE) / 2 + 2,
          lines,
          msgStyle,
        ),
      );
      badgeX = x0;
      if (s.activation === "+") begin(s.to, arrowY + 4);
      if (s.activation === "-") finish(s.from, arrowY + 22);
      y += rowH + 22;
    } else {
      y += 8 + textH;
      arrowY = y + 4;
      if (s.activation === "+") begin(s.to, arrowY);
      const x1 = edgeX(s.from, dir);
      let x2 = edgeX(s.to, -dir);
      const target = headers[to];
      if (s.created && target)
        x2 = (centers[to] ?? 0) - dir * (target.width / 2 + (target.p.kind === "actor" ? -6 : 0));
      front.push(line(x1, arrowY, x2, arrowY, stroke));
      const mid = (x1 + x2) / 2;
      front.push(
        textLines(mid, arrowY - 10 - (lines.length - 1) * LINE, lines, {
          ...msgStyle,
          anchor: "middle",
        }),
      );
      badgeX = x1;
      if (s.activation === "-") finish(s.from, arrowY);
      y = arrowY + 14;
    }
    if (model.autonumber) {
      const bx = badgeX + (self ? 0 : dir * 10);
      front.push(circle(bx, arrowY, 8, { fill: color.text }));
      front.push(
        text(bx, arrowY, String(counter), {
          size: 10,
          weight: "600",
          anchor: "middle",
          fill: color.canvas,
        }),
      );
      counter += model.autonumber.step;
    }
    if (s.destroyed !== undefined) {
      const gx = cx(s.destroyed);
      ended.set(s.destroyed, arrowY);
      front.push(
        path(
          `M${n(gx - 8)},${n(arrowY - 8)} L${n(gx + 8)},${n(arrowY + 8)} M${n(gx + 8)},${n(arrowY - 8)} L${n(gx - 8)},${n(arrowY + 8)}`,
          { stroke: color.crit, "stroke-width": 2.5 },
        ),
      );
    }
    touch(
      Math.min(centers[from] ?? 0, centers[to] ?? 0) - 20,
      Math.max(centers[from] ?? 0, centers[to] ?? 0) +
        (self ? SELF_W + 24 + measureLines(lines, msgStyle).width : 20),
    );
  };

  const note = (s: Extract<SequenceStatement, { kind: "note" }>): void => {
    const lines = wrapText(s.text, NOTE_WRAP, noteStyle);
    const size = measureLines(lines, noteStyle);
    const first = cx(s.participants[0] ?? "");
    let width = size.width + 20;
    let x: number;
    if (s.placement === "left") x = first - 12 - width;
    else if (s.placement === "right") x = first + 12;
    else if (s.participants.length === 2) {
      const a = Math.min(...s.participants.map(cx));
      const b = Math.max(...s.participants.map(cx));
      width = Math.max(width, b - a + 20);
      x = (a + b) / 2 - width / 2;
    } else x = first - width / 2;
    const height = size.height + 14;
    y += 6;
    front.push(
      rect(x, y, width, height, {
        rx: 3,
        fill: color.groupHdr,
        stroke: color.nodeStroke,
        "stroke-width": 1,
      }) + textLines(x + 10, y + 7 + noteStyle.size * 0.65, lines, noteStyle),
    );
    touch(x - 8, x + width + 8);
    y += height + 8;
  };

  const walk = (statements: SequenceStatement[]): void => {
    for (const s of statements) {
      budget.tick();
      if (s.kind === "message") message(s);
      else if (s.kind === "note") note(s);
      else if (s.kind === "activate") begin(s.id, y);
      else if (s.kind === "deactivate") finish(s.id, y);
      else {
        y += 6;
        const top = y;
        const own = s.block === "rect";
        y += own ? 4 : 24;
        const dividers: number[] = [];
        const labels: { y: number; text: string }[] = [];
        s.sections.forEach((section, k) => {
          if (k > 0) {
            dividers.push(y - 4);
            y += 4;
          }
          if (section.label && !own) {
            labels.push({ y: k === 0 ? top + 12 : y + 8, text: section.label });
            if (k > 0) y += 20;
          }
          walk(section.statements);
          y += 4;
        });
        y += 6;
        const range = s.sections
          .map((sec) => extentOf(sec.statements))
          .reduce<[number, number] | undefined>(
            (acc, e) => (e ? (acc ? [Math.min(acc[0], e[0]), Math.max(acc[1], e[1])] : e) : acc),
            undefined,
          ) ?? [centers[0] ?? 0, centers[count - 1] ?? 0];
        const tabW = own ? 0 : textWidth(s.block, { size: FONT.small, bold: true }) + 22;
        const x1 = range[0] - 18;
        const x2 = Math.max(range[1] + 18, x1 + tabW + 40);
        const bottom = y;
        back.push(
          rect(x1, top, x2 - x1, bottom - top, {
            rx: 2,
            fill: own ? color.info : "none",
            "fill-opacity": own ? 0.1 : undefined,
            stroke: own ? "none" : color.textFaint,
            "stroke-width": 1.2,
          }),
        );
        for (const dy of dividers) {
          back.push(
            line(x1, dy, x2, dy, {
              stroke: color.textFaint,
              "stroke-width": 1,
              "stroke-dasharray": "5 4",
            }),
          );
        }
        for (const item of labels) {
          const label = fitWidth(item.text, Math.max(60, x2 - x1 - tabW - 20), {
            size: FONT.small + 1,
          });
          chrome.push(
            text(x1 + tabW + 8, item.y, `[${label}]`, {
              size: FONT.small + 1,
              fill: color.textSec,
            }),
          );
        }
        if (!own) {
          chrome.push(
            path(
              `M${n(x1)},${n(top)} L${n(x1 + tabW)},${n(top)} L${n(x1 + tabW)},${n(top + 14)} L${n(x1 + tabW - 8)},${n(top + 22)} L${n(x1)},${n(top + 22)} Z`,
              {
                fill: color.groupHdr,
                stroke: color.textFaint,
                "stroke-width": 1.2,
                "stroke-linejoin": "round",
              },
            ),
          );
          chrome.push(
            text(x1 + 8, top + 11, s.block, { size: FONT.small, weight: "700", fill: color.text }),
          );
        }
        touch(x1 - 8, x2 + 8);
        y += 6;
      }
    }
  };
  walk(model.statements);
  for (const [id, stack] of open)
    for (const start of stack.splice(0)) bars.push({ x: barX(id, 0), y1: start, y2: y });
  const footerTop = y + 18;

  // Assemble layers.
  const parts: string[] = [];
  model.boxes.forEach((box) => {
    const members = box.participants.map(at);
    if (members.length === 0) return;
    const lo = Math.min(...members);
    const hi = Math.max(...members);
    const x1 = (centers[lo] ?? 0) - (headers[lo]?.width ?? 0) / 2 - 10;
    const x2 = (centers[hi] ?? 0) + (headers[hi]?.width ?? 0) / 2 + 10;
    const top = headerTop - 26;
    parts.push(
      rect(x1, top, x2 - x1, footerTop + headerHeight + 10 - top, {
        rx: 6,
        fill: color.groupFill,
        stroke: color.innerStroke,
        "stroke-width": 1,
      }),
    );
    parts.push(
      text(
        (x1 + x2) / 2,
        top + 11,
        fitWidth(box.label, x2 - x1 - 12, { size: FONT.small, bold: true }),
        { size: FONT.small, weight: "600", anchor: "middle", fill: color.textSec },
      ),
    );
  });
  parts.push(...back);
  headers.forEach((h, i) => {
    const c = centers[i] ?? 0;
    const start = created.get(h.p.id);
    const top0 = start === undefined ? headerTop + headerHeight - h.height : start - h.height / 2;
    const lifeStart = top0 + h.height;
    const lifeEnd = ended.get(h.p.id) ?? footerTop + (headerHeight - h.height);
    parts.push(
      line(c, lifeStart, c, lifeEnd, {
        stroke: color.textFaint,
        "stroke-width": 1,
        "stroke-dasharray": "4 4",
      }),
    );
    parts.push(drawHeader(h, c, top0));
    if (!ended.has(h.p.id)) parts.push(drawHeader(h, c, footerTop + (headerHeight - h.height)));
  });
  for (const bar of bars) {
    parts.push(
      rect(bar.x, bar.y1, BAR, bar.y2 - bar.y1, {
        fill: color.nodeFill,
        stroke: color.nodeStroke,
        "stroke-width": 1,
      }),
    );
  }
  parts.push(...chrome, ...front);
  if (titleLines.length) {
    parts.push(
      textLines((minX + maxX) / 2, MARGIN + 8, titleLines, {
        size: FONT.title,
        weight: "700",
        anchor: "middle",
      }),
    );
  }
  const height = footerTop + headerHeight + MARGIN;
  const width = maxX - minX;
  const defs = markerDefs(["arrow-filled", "arrow-open", "arrow-cross"]);
  return svgDoc(
    width,
    height,
    el("g", { transform: `translate(${n(-minX)},0)` }, parts.join("")),
    defs,
  );
}
