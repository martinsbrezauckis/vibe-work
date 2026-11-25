/**
 * ZenUML sequence diagram drawing: participant headers (with actor, database, and annotator
 * glyphs), lifelines (with delayed `new` creation headers), activation bars over nested sync
 * calls, solid sync/async and dashed reply arrows, // comment callouts, and control-flow frames.
 */
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
} from "../../svg";
import { FONT, color } from "../../theme";
import type { ZenParticipant, ZenStatement, ZenumlModel } from "./parse";

const MARGIN = 24;
const MIN_GAP = 128;
const MSG_WRAP = 260;
const LINE = FONT.body * 1.3;
const BAR = 10;
const SELF_W = 32;
const ENTRY_LEN = 66;

interface HeaderBox {
  p: ZenParticipant;
  width: number;
  height: number;
  lines: string[];
}

function headerOf(p: ZenParticipant): HeaderBox {
  const style = { size: FONT.body, bold: true };
  const lines = wrapText(p.label, 170, style, 3);
  const size = measureLines(lines, style, 1.3);
  if (p.annotator === "actor") {
    return {
      p,
      lines,
      width: Math.max(60, size.width + 12),
      height: 54 + size.height,
    };
  }
  const extraH = p.annotator ? 16 : 0;
  const tagW = p.annotator ? textWidth(`«${p.annotator}»`, { size: FONT.small }) + 20 : 0;
  return {
    p,
    lines,
    width: Math.max(88, size.width + 28, tagW),
    height: Math.max(40, size.height + 18 + extraH),
  };
}

function drawHeader(h: HeaderBox, cx: number, top: number): string {
  const style = { size: FONT.body, weight: "600" as const, anchor: "middle" as const };
  if (h.p.annotator === "actor") {
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
  let out = rect(cx - h.width / 2, top, h.width, h.height, {
    rx: h.p.annotator === "database" ? 10 : 5,
    fill: color.nodeFill,
    stroke: color.nodeStroke,
    "stroke-width": 1.1,
  });
  let textStart = top + h.height / 2 - ((h.lines.length - 1) * LINE) / 2;
  if (h.p.annotator) {
    out += text(cx, top + 11, `«${h.p.annotator}»`, {
      size: FONT.small,
      anchor: "middle",
      fill: color.textSec,
    });
    textStart = top + 24 + ((h.lines.length - 1) * LINE) / 2;
  }
  out += textLines(cx, textStart, h.lines, style);
  return out;
}

/** Draw a parsed ZenUML diagram as a complete SVG string. */
export function drawZenuml(model: ZenumlModel): string {
  const budget = new StepBudget();
  const headers = model.participants.map(headerOf);
  const index = new Map(model.participants.map((p, i) => [p.id, i]));
  const at = (id: string): number => index.get(id) ?? 0;

  const msgStyle = { size: FONT.body };
  const noteStyle = { size: FONT.small + 1 };
  const count = headers.length;
  const gap = headers.map((h, i) => {
    const next = headers[i + 1];
    return next ? Math.max(MIN_GAP, h.width / 2 + next.width / 2 + 26) : 0;
  });

  let leftRoom = MARGIN + (headers[0]?.width ?? 0) / 2;
  let rightRoom = MARGIN + (headers[count - 1]?.width ?? 0) / 2;
  const spans: { from: number; to: number; need: number }[] = [];
  const needGap = (i: number, need: number): void => {
    gap[i] = Math.max(gap[i] ?? 0, need);
  };

  const measureStmts = (stmts: readonly ZenStatement[]): void => {
    for (const s of stmts) {
      budget.tick();
      if (s.kind === "sync") {
        const label = s.assign ? `${s.assign} = ${s.method}` : s.method;
        const w = measureLines(wrapText(label, MSG_WRAP, msgStyle), msgStyle).width;
        const b = at(s.to);
        if (s.from === undefined) {
          if (b === 0) leftRoom = Math.max(leftRoom, w + ENTRY_LEN + 20);
          else needGap(b - 1, w + ENTRY_LEN + 20);
        } else {
          const a = at(s.from);
          if (a === b) {
            if (a === count - 1) rightRoom = Math.max(rightRoom, w + SELF_W + 24);
            else needGap(a, w + SELF_W + 24);
          } else {
            spans.push({ from: Math.min(a, b), to: Math.max(a, b), need: w + 44 });
          }
        }
        measureStmts(s.body);
      } else if (s.kind === "async") {
        const w = measureLines(wrapText(s.text, MSG_WRAP, msgStyle), msgStyle).width;
        const a = at(s.from);
        const b = at(s.to);
        if (a === b) {
          if (a === count - 1) rightRoom = Math.max(rightRoom, w + SELF_W + 24);
          else needGap(a, w + SELF_W + 24);
        } else {
          spans.push({ from: Math.min(a, b), to: Math.max(a, b), need: w + 44 });
        }
      } else if (s.kind === "new") {
        const label = `${s.assign ? `${s.assign} = ` : ""}new ${s.target}(${s.args})`;
        const w = measureLines(wrapText(label, MSG_WRAP, msgStyle), msgStyle).width;
        const b = at(s.target);
        if (s.from === undefined) {
          if (b === 0) leftRoom = Math.max(leftRoom, w + ENTRY_LEN + 20);
          else needGap(b - 1, w + ENTRY_LEN + 20);
        } else {
          const a = at(s.from);
          spans.push({ from: Math.min(a, b), to: Math.max(a, b), need: w + 64 });
        }
        measureStmts(s.body);
      } else if (s.kind === "return") {
        const w = measureLines(wrapText(s.value, MSG_WRAP, msgStyle), msgStyle).width;
        const a = at(s.from);
        if (s.to === undefined) {
          if (a === 0) leftRoom = Math.max(leftRoom, w + ENTRY_LEN + 20);
          else needGap(a - 1, w + ENTRY_LEN + 20);
        } else {
          const b = at(s.to);
          spans.push({ from: Math.min(a, b), to: Math.max(a, b), need: w + 44 });
        }
      } else if (s.kind === "loop" || s.kind === "opt" || s.kind === "par") {
        measureStmts(s.body);
      } else if (s.kind === "alt") {
        for (const br of s.branches) measureStmts(br.body);
      } else {
        measureStmts(s.tryBody);
        if (s.catchBody) measureStmts(s.catchBody);
        if (s.finallyBody) measureStmts(s.finallyBody);
      }
    }
  };
  measureStmts(model.statements);

  spans.sort((x, y) => x.to - x.from - (y.to - y.from));
  for (const { from, to, need } of spans) {
    let have = 0;
    for (let i = from; i < to; i++) have += gap[i] ?? 0;
    if (have < need) {
      for (let i = from; i < to; i++) gap[i] = (gap[i] ?? 0) + (need - have) / (to - from);
    }
  }

  const centers: number[] = [];
  let cursor = leftRoom;
  headers.forEach((_, i) => {
    centers.push(cursor);
    cursor += gap[i] ?? 0;
  });
  const cx = (id: string): number => centers[at(id)] ?? 0;

  const titleLines = model.title
    ? wrapText(model.title, 520, { size: FONT.title, bold: true }, 2)
    : [];
  const titleH = titleLines.length ? titleLines.length * FONT.title * 1.3 + 14 : 0;
  const headerTop = MARGIN + titleH;
  const headerH = Math.max(0, ...headers.map((h) => h.height));
  let y = headerTop + headerH + 22;
  let minX = 0;
  let maxX = Math.max(0, cursor - (gap[count - 1] ?? 0) + rightRoom);
  const touch = (x1: number, x2: number): void => {
    minX = Math.min(minX, x1);
    maxX = Math.max(maxX, x2);
  };
  centers.forEach((c, i) => {
    const w = headers[i]?.width ?? 0;
    touch(c - w / 2 - MARGIN, c + w / 2 + MARGIN);
  });

  const back: string[] = [];
  const chrome: string[] = [];
  const front: string[] = [];
  const bars: { x: number; y1: number; y2: number }[] = [];
  const createdAt = new Map<string, number>();
  const activeDepth = new Map<string, number>();

  const barX = (id: string, depth: number): number => cx(id) - BAR / 2 + depth * (BAR / 2);

  const drawComment = (comment: string | undefined, anchorX: number): void => {
    if (!comment) return;
    const lines = wrapText(comment, 220, noteStyle, 4);
    const size = measureLines(lines, noteStyle);
    const w = size.width + 18;
    const h = size.height + 10;
    const x = anchorX - w / 2;
    y += 4;
    front.push(
      rect(x, y, w, h, {
        rx: 4,
        fill: color.groupHdr,
        stroke: color.innerStroke,
        "stroke-width": 1,
      }) + textLines(x + 9, y + 5 + noteStyle.size * 0.65, lines, noteStyle),
    );
    touch(x - 8, x + w + 8);
    y += h + 6;
  };

  const drawArrow = (
    fromX: number,
    toX: number,
    label: string,
    dashed: boolean,
    openHead: boolean,
  ): void => {
    const lines = wrapText(label, MSG_WRAP, msgStyle);
    const textH = lines.length * LINE;
    const self = Math.abs(fromX - toX) < 0.5;
    const stroke = {
      stroke: color.line,
      "stroke-width": 1.4,
      "stroke-dasharray": dashed ? "6 4" : undefined,
      "marker-end": markerRef(openHead ? "arrow-open" : "arrow-filled"),
      fill: "none",
    };
    if (self) {
      y += 8;
      const rowH = Math.max(24, textH);
      const ay = y + rowH / 2 - 12;
      front.push(
        path(
          `M${n(fromX)},${n(ay)} L${n(fromX + SELF_W)},${n(ay)} L${n(fromX + SELF_W)},${n(ay + 22)} L${n(fromX + 1)},${n(ay + 22)}`,
          stroke,
        ),
      );
      front.push(
        textLines(
          fromX + SELF_W + 10,
          y + rowH / 2 - ((lines.length - 1) * LINE) / 2 + 2,
          lines,
          msgStyle,
        ),
      );
      touch(fromX - 16, fromX + SELF_W + 24 + measureLines(lines, msgStyle).width);
      y += rowH + 22;
    } else {
      y += 8 + textH;
      const ay = y + 4;
      front.push(line(fromX, ay, toX, ay, stroke));
      front.push(
        textLines((fromX + toX) / 2, ay - 10 - (lines.length - 1) * LINE, lines, {
          ...msgStyle,
          anchor: "middle",
        }),
      );
      touch(Math.min(fromX, toX) - 18, Math.max(fromX, toX) + 18);
      y = ay + 14;
    }
  };

  const extentOf = (stmts: readonly ZenStatement[]): [number, number] | undefined => {
    let lo = Number.POSITIVE_INFINITY;
    let hi = Number.NEGATIVE_INFINITY;
    const add = (a: number, b: number): void => {
      lo = Math.min(lo, a);
      hi = Math.max(hi, b);
    };
    for (const s of stmts) {
      if (s.kind === "sync") {
        const tx = cx(s.to);
        add(s.from ? Math.min(cx(s.from), tx) : tx - ENTRY_LEN, tx);
        const inner = extentOf(s.body);
        if (inner) add(inner[0], inner[1]);
      } else if (s.kind === "async") {
        add(Math.min(cx(s.from), cx(s.to)), Math.max(cx(s.from), cx(s.to)));
      } else if (s.kind === "new") {
        const tx = cx(s.target);
        add(s.from ? Math.min(cx(s.from), tx) : tx - ENTRY_LEN, tx);
        const inner = extentOf(s.body);
        if (inner) add(inner[0], inner[1]);
      } else if (s.kind === "return") {
        const fx = cx(s.from);
        add(s.to ? Math.min(fx, cx(s.to)) : fx - ENTRY_LEN, fx);
      } else if (s.kind === "loop" || s.kind === "opt" || s.kind === "par") {
        const inner = extentOf(s.body);
        if (inner) add(inner[0] - 12, inner[1] + 12);
      } else if (s.kind === "alt") {
        for (const br of s.branches) {
          const inner = extentOf(br.body);
          if (inner) add(inner[0] - 12, inner[1] + 12);
        }
      } else {
        for (const part of [s.tryBody, s.catchBody ?? [], s.finallyBody ?? []]) {
          const inner = extentOf(part);
          if (inner) add(inner[0] - 12, inner[1] + 12);
        }
      }
    }
    return lo <= hi ? [lo, hi] : undefined;
  };

  const drawFrame = (
    tab: string,
    sections: { label: string; body: readonly ZenStatement[] }[],
  ): void => {
    y += 6;
    const top = y;
    y += 24;
    const dividers: number[] = [];
    const labels: { y: number; text: string }[] = [];
    sections.forEach((sec, k) => {
      if (k > 0) {
        dividers.push(y - 4);
        y += 4;
      }
      if (sec.label) {
        labels.push({ y: k === 0 ? top + 12 : y + 8, text: sec.label });
        if (k > 0) y += 20;
      }
      walk(sec.body);
      y += 4;
    });
    y += 6;
    const range = sections
      .map((sec) => extentOf(sec.body))
      .reduce<[number, number] | undefined>(
        (acc, e) => (e ? (acc ? [Math.min(acc[0], e[0]), Math.max(acc[1], e[1])] : e) : acc),
        undefined,
      ) ?? [centers[0] ?? 0, centers[count - 1] ?? 0];
    const tabW = textWidth(tab, { size: FONT.small, bold: true }) + 22;
    const x1 = range[0] - 18;
    const x2 = Math.max(range[1] + 18, x1 + tabW + 60);
    back.push(
      rect(x1, top, x2 - x1, y - top, {
        rx: 2,
        fill: "none",
        stroke: color.textFaint,
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
      const lbl = fitWidth(item.text, Math.max(60, x2 - x1 - tabW - 20), { size: FONT.small + 1 });
      chrome.push(
        text(x1 + tabW + 8, item.y, `[${lbl}]`, {
          size: FONT.small + 1,
          fill: color.textSec,
        }),
      );
    }
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
    chrome.push(text(x1 + 8, top + 11, tab, { size: FONT.small, weight: "700", fill: color.text }));
    touch(x1 - 8, x2 + 8);
    y += 6;
  };

  const walk = (stmts: readonly ZenStatement[]): void => {
    for (const s of stmts) {
      budget.tick();
      if (s.kind === "sync") {
        const tx = cx(s.to);
        const fx = s.from !== undefined ? cx(s.from) : tx - ENTRY_LEN;
        drawComment(s.comment, (fx + tx) / 2);
        const label = s.assign ? `${s.assign} = ${s.method}` : s.method;
        const depth = activeDepth.get(s.to) ?? 0;
        const barStart = y + 12;
        drawArrow(fx, tx, label, false, false);
        if (s.body.length > 0) {
          activeDepth.set(s.to, depth + 1);
          walk(s.body);
          activeDepth.set(s.to, depth);
          bars.push({ x: barX(s.to, depth), y1: barStart, y2: Math.max(y, barStart + 14) });
        } else {
          bars.push({ x: barX(s.to, depth), y1: barStart, y2: barStart + 16 });
        }
      } else if (s.kind === "async") {
        const fx = cx(s.from);
        const tx = cx(s.to);
        drawComment(s.comment, (fx + tx) / 2);
        drawArrow(fx, tx, s.text, s.isReply, s.isReply);
      } else if (s.kind === "new") {
        const targetHeader = headers[at(s.target)];
        const tx = cx(s.target);
        const fx = s.from !== undefined ? cx(s.from) : tx - ENTRY_LEN;
        drawComment(s.comment, (fx + tx) / 2);
        if (targetHeader && !createdAt.has(s.target)) {
          createdAt.set(s.target, y + targetHeader.height / 2);
          y += Math.max(0, targetHeader.height / 2 - 10);
        }
        const label = `${s.assign ? `${s.assign} = ` : ""}new ${s.target}${s.args ? `(${s.args})` : ""}`;
        const stopX =
          targetHeader && tx > fx
            ? tx - targetHeader.width / 2
            : targetHeader && tx < fx
              ? tx + targetHeader.width / 2
              : tx;
        const depth = activeDepth.get(s.target) ?? 0;
        const barStart = y + 12;
        drawArrow(fx, stopX, label, false, false);
        if (s.body.length > 0) {
          activeDepth.set(s.target, depth + 1);
          walk(s.body);
          activeDepth.set(s.target, depth);
          bars.push({ x: barX(s.target, depth), y1: barStart, y2: Math.max(y, barStart + 14) });
        }
      } else if (s.kind === "return") {
        const fx = cx(s.from);
        const tx = s.to !== undefined ? cx(s.to) : fx - ENTRY_LEN;
        drawComment(s.comment, (fx + tx) / 2);
        drawArrow(fx, tx, s.value || "return", true, true);
      } else if (s.kind === "loop") {
        drawComment(s.comment, centers[0] ?? 0);
        drawFrame(s.loopKind, [{ label: s.condition, body: s.body }]);
      } else if (s.kind === "alt") {
        drawComment(s.comment, centers[0] ?? 0);
        drawFrame("alt", s.branches);
      } else if (s.kind === "opt") {
        drawComment(s.comment, centers[0] ?? 0);
        drawFrame("opt", [{ label: s.label, body: s.body }]);
      } else if (s.kind === "par") {
        drawComment(s.comment, centers[0] ?? 0);
        drawFrame(
          "par",
          s.body.length > 1
            ? s.body.map((item) => ({ label: "", body: [item] }))
            : [{ label: "", body: s.body }],
        );
      } else {
        drawComment(s.comment, centers[0] ?? 0);
        const sections: { label: string; body: readonly ZenStatement[] }[] = [
          { label: "try", body: s.tryBody },
        ];
        if (s.catchBody) sections.push({ label: "catch", body: s.catchBody });
        if (s.finallyBody) sections.push({ label: "finally", body: s.finallyBody });
        drawFrame("try", sections);
      }
    }
  };
  walk(model.statements);

  const footerTop = y + 18;
  const parts: string[] = [...back];

  headers.forEach((h, i) => {
    const c = centers[i] ?? 0;
    const createdY = createdAt.get(h.p.id);
    const top0 = createdY === undefined ? headerTop + headerH - h.height : createdY - h.height / 2;
    const lifeStart = top0 + h.height;
    const lifeEnd = footerTop + (headerH - h.height);
    parts.push(
      line(c, lifeStart, c, lifeEnd, {
        stroke: color.textFaint,
        "stroke-width": 1,
        "stroke-dasharray": "4 4",
      }),
    );
    parts.push(drawHeader(h, c, top0));
    parts.push(drawHeader(h, c, footerTop + (headerH - h.height)));
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
  if (titleLines.length > 0) {
    parts.push(
      textLines((minX + maxX) / 2, MARGIN + 8, titleLines, {
        size: FONT.title,
        weight: "700",
        anchor: "middle",
      }),
    );
  }

  return svgDoc(
    maxX - minX,
    footerTop + headerH + MARGIN,
    el("g", { transform: `translate(${n(-minX)},0)` }, parts.join("")),
    markerDefs(["arrow-filled", "arrow-open"]),
  );
}
