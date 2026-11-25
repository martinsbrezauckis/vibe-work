/**
 * GitGraph diagram drawing: LR, TB, and BT orientations with branch lanes in resolved order,
 * cyclic 8-colour branch palette, smooth fork/merge/cherry-pick paths, commit type glyphs
 * (normal, reverse, highlight, merge double-circle, cherry emblem), labels, and tag pills.
 */
import { StepBudget } from "../../limits";
import { clampLabel, fitWidth, textWidth } from "../../measure";
import { circle, el, esc, g, line, n, path, rect, svgDoc, text } from "../../svg";
import { FONT, color, series } from "../../theme";
import type { GitBranch, GitCommit, GitGraphModel } from "./parse";

const PAD = 24;
const COMMIT_R = 9;
const COMMIT_STEP_LR = 64;
const COMMIT_STEP_TB = 52;
const LANE_GAP_LR = 78;
const LANE_GAP_TB = 114;

interface Point {
  x: number;
  y: number;
}

function rotatedText(
  x: number,
  y: number,
  content: string,
  angle: number,
  anchor: "start" | "middle" | "end",
  fill: string,
): string {
  return el(
    "text",
    {
      x,
      y,
      dy: "0.35em",
      "text-anchor": anchor,
      "font-size": FONT.small,
      "font-weight": "500",
      fill,
      transform: `rotate(${n(angle)},${n(x)},${n(y)})`,
    },
    esc(content),
  );
}

function drawBranchPill(cx: number, cy: number, name: string, tone: string, maxW: number): string {
  const style = { size: FONT.small, bold: true };
  const label = fitWidth(clampLabel(name), maxW - 16, style);
  const w = Math.max(46, Math.ceil(textWidth(label, style) + 18));
  const h = 24;
  return (
    rect(cx - w / 2, cy - h / 2, w, h, {
      rx: 12,
      fill: tone,
      "fill-opacity": 0.18,
      stroke: tone,
      "stroke-width": 1.3,
    }) +
    text(cx, cy, label, {
      size: FONT.small,
      weight: "700",
      anchor: "middle",
      fill: color.text,
    })
  );
}

function drawTagPill(cx: number, cy: number, tagText: string, tone: string, maxW: number): string {
  const style = { size: FONT.small };
  const label = fitWidth(clampLabel(tagText), maxW, style);
  const w = Math.max(28, Math.ceil(textWidth(label, style) + 14));
  const h = 18;
  return (
    rect(cx - w / 2, cy - h / 2, w, h, {
      rx: 4,
      fill: color.groupHdr,
      stroke: tone,
      "stroke-width": 1.1,
    }) +
    text(cx, cy, label, {
      size: FONT.small,
      weight: "500",
      anchor: "middle",
      fill: color.text,
    })
  );
}

function drawCommitNode(p: Point, commit: GitCommit, tone: string): string {
  let out = "";
  if (commit.isMerge) {
    out += circle(p.x, p.y, COMMIT_R + 3.5, {
      fill: color.canvas,
      stroke: tone,
      "stroke-width": 2,
    });
  }
  if (commit.type === "HIGHLIGHT") {
    const r = commit.isMerge ? COMMIT_R - 1.5 : COMMIT_R;
    out += rect(p.x - r, p.y - r, r * 2, r * 2, {
      rx: 3,
      fill: tone,
      stroke: color.text,
      "stroke-width": 1.8,
    });
  } else {
    const r = commit.isMerge ? COMMIT_R - 1 : COMMIT_R;
    out += circle(p.x, p.y, r, {
      fill: tone,
      stroke: commit.isMerge ? tone : color.canvas,
      "stroke-width": commit.isMerge ? 1.4 : 2,
    });
    if (commit.type === "REVERSE") {
      const k = 4.2;
      out += path(
        `M${n(p.x - k)},${n(p.y - k)} L${n(p.x + k)},${n(p.y + k)} M${n(p.x + k)},${n(p.y - k)} L${n(p.x - k)},${n(p.y + k)}`,
        {
          fill: "none",
          stroke: color.canvas,
          "stroke-width": 2,
          "stroke-linecap": "round",
        },
      );
    }
  }
  if (commit.cherryPickFrom) {
    out += path(
      `M${n(p.x - 2.5)},${n(p.y + 1.5)} Q${n(p.x)},${n(p.y - 5)} ${n(p.x + 2.5)},${n(p.y + 1.5)}`,
      {
        fill: "none",
        stroke: color.canvas,
        "stroke-width": 1.3,
        "stroke-linecap": "round",
      },
    );
    out += circle(p.x - 2.5, p.y + 2.5, 2, { fill: color.canvas });
    out += circle(p.x + 2.5, p.y + 2.5, 2, { fill: color.canvas });
  }
  return g({}, out);
}

function connectPath(
  from: Point,
  to: Point,
  horizontal: boolean,
  tone: string,
  dashed = false,
): string {
  const attrs = {
    fill: "none",
    stroke: tone,
    "stroke-width": dashed ? 1.6 : 2.2,
    "stroke-linecap": "round" as const,
    ...(dashed ? { "stroke-dasharray": "4 3" } : {}),
  };
  if (
    (horizontal && Math.abs(from.y - to.y) < 0.5) ||
    (!horizontal && Math.abs(from.x - to.x) < 0.5)
  ) {
    return line(from.x, from.y, to.x, to.y, attrs);
  }
  const d = horizontal
    ? `M${n(from.x)},${n(from.y)} C${n((from.x + to.x) / 2)},${n(from.y)} ${n((from.x + to.x) / 2)},${n(to.y)} ${n(to.x)},${n(to.y)}`
    : `M${n(from.x)},${n(from.y)} C${n(from.x)},${n((from.y + to.y) / 2)} ${n(to.x)},${n((from.y + to.y) / 2)} ${n(to.x)},${n(to.y)}`;
  return path(d, attrs);
}

/** Draw a parsed gitGraph diagram (LR, TB, or BT) as a complete SVG string. */
export function drawGitGraph(model: GitGraphModel): string {
  const budget = new StepBudget();
  const horizontal = model.direction === "LR";
  const branchMap = new Map<string, { branch: GitBranch; lane: number }>();
  model.branches.forEach((branch, lane) => {
    branchMap.set(branch.name, { branch, lane });
  });

  const branchColor = (name: string): string => series(branchMap.get(name)?.branch.colorIndex ?? 0);
  const branchLane = (name: string): number => branchMap.get(name)?.lane ?? 0;

  const commitStyle = { size: FONT.small };
  const branchLabelW = Math.min(
    140,
    Math.max(
      72,
      ...model.branches.map(
        (b) => textWidth(clampLabel(b.name), { size: FONT.small, bold: true }) + 22,
      ),
    ),
  );

  const commitPos: Point[] = [];
  let width: number;
  let height: number;
  let guides = "";
  let headers = "";
  let labels = "";

  if (horizontal) {
    const startX = Math.max(PAD + branchLabelW + 28, PAD + 126);
    const step = Math.min(
      COMMIT_STEP_LR,
      Math.max(44, Math.floor((15_200 - startX - PAD * 2) / Math.max(1, model.commits.length))),
    );
    const topRoom = 30;
    const bottomRoom = 92;
    const laneY = (lane: number): number => PAD + topRoom + lane * LANE_GAP_LR;
    const endX = startX + Math.max(0, model.commits.length - 1) * step;
    width = Math.max(endX + PAD + 48, startX + 80);
    height =
      PAD + topRoom + Math.max(0, model.branches.length - 1) * LANE_GAP_LR + bottomRoom + PAD;

    for (let i = 0; i < model.branches.length; i++) {
      budget.tick();
      const branch = model.branches[i];
      if (!branch) continue;
      const y = laneY(i);
      const tone = series(branch.colorIndex);
      const lineStart = startX - 28;
      guides += line(lineStart, y, width - PAD, y, {
        stroke: color.innerStroke,
        "stroke-width": 1,
        "stroke-dasharray": "3 4",
      });
      headers += drawBranchPill(lineStart - branchLabelW / 2, y, branch.name, tone, branchLabelW);
    }

    model.commits.forEach((commit, idx) => {
      budget.tick();
      const x = startX + idx * step;
      const y = laneY(branchLane(commit.branch));
      commitPos[commit.seq] = { x, y };
      const tone = branchColor(commit.branch);
      if (commit.tag) {
        labels += drawTagPill(x, y - 22, commit.tag, tone, Math.max(84, step + 30));
      }
      const idText = fitWidth(clampLabel(commit.id), 145, commitStyle);
      labels += rotatedText(x, y + 18, idText, -35, "end", color.textSec);
    });
  } else {
    const laneGap = Math.min(
      LANE_GAP_TB,
      Math.max(88, Math.floor((15_200 - PAD * 2) / Math.max(1, model.branches.length))),
    );
    const startLaneX = PAD + laneGap / 2;
    const headerBand = 44;
    const step = Math.min(
      COMMIT_STEP_TB,
      Math.max(
        38,
        Math.floor((15_200 - PAD * 2 - headerBand - 40) / Math.max(1, model.commits.length)),
      ),
    );
    const laneX = (lane: number): number => startLaneX + lane * laneGap;
    const bottomToTop = model.direction === "BT";
    const firstCommitY = PAD + (bottomToTop ? 28 : headerBand + 26);
    const commitY = (idx: number): number =>
      bottomToTop
        ? firstCommitY + (model.commits.length - 1 - idx) * step
        : firstCommitY + idx * step;

    const lanesRight = startLaneX + Math.max(0, model.branches.length - 1) * laneGap + laneGap / 2;
    width = Math.max(lanesRight + PAD + 44, 240);
    const lastCommitBottom = firstCommitY + Math.max(0, model.commits.length - 1) * step;
    height = lastCommitBottom + (bottomToTop ? headerBand + 26 : 36) + PAD;

    for (let i = 0; i < model.branches.length; i++) {
      budget.tick();
      const branch = model.branches[i];
      if (!branch) continue;
      const x = laneX(i);
      const tone = series(branch.colorIndex);
      const lineTop = bottomToTop ? PAD + 12 : PAD + headerBand;
      const lineBottom = bottomToTop ? height - PAD - headerBand : height - PAD;
      guides += line(x, lineTop, x, lineBottom, {
        stroke: color.innerStroke,
        "stroke-width": 1,
        "stroke-dasharray": "3 4",
      });
      const pillY = bottomToTop ? height - PAD - 14 : PAD + 14;
      headers += drawBranchPill(x, pillY, branch.name, tone, laneGap - 8);
    }

    model.commits.forEach((commit, idx) => {
      budget.tick();
      const x = laneX(branchLane(commit.branch));
      const y = commitY(idx);
      commitPos[commit.seq] = { x, y };
      const tone = branchColor(commit.branch);
      const idText = fitWidth(clampLabel(commit.id), laneGap - 22, commitStyle);
      labels += text(x + 14, y, idText, { size: FONT.small, fill: color.textSec });
      if (commit.tag) {
        labels += drawTagPill(x, y - 20, commit.tag, tone, laneGap - 10);
      }
    });
  }

  let edges = "";
  for (const commit of model.commits) {
    budget.tick();
    const to = commitPos[commit.seq];
    if (!to) continue;
    commit.parentSeqs.forEach((parentSeq, pIdx) => {
      const from = commitPos[parentSeq];
      if (!from) return;
      const parentCommit = model.commits[parentSeq];
      const edgeColor =
        pIdx === 0
          ? branchColor(commit.branch)
          : branchColor(parentCommit?.branch ?? commit.branch);
      edges += connectPath(from, to, horizontal, edgeColor);
    });
    if (commit.cherryPickFromSeq !== undefined) {
      const src = commitPos[commit.cherryPickFromSeq];
      if (src) {
        edges += connectPath(src, to, horizontal, color.accent, true);
      }
    }
  }

  let nodes = "";
  for (const commit of model.commits) {
    budget.tick();
    const p = commitPos[commit.seq];
    if (!p) continue;
    nodes += drawCommitNode(p, commit, branchColor(commit.branch));
  }

  return svgDoc(width, height, g({}, guides + edges + headers + nodes + labels));
}
