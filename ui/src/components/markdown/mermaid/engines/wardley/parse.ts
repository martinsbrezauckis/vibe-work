/**
 * Wardley Map parser. Grammar follows Mermaid's wardley page: title, size, custom evolution
 * stages (with dual labels and @boundaries), anchors, components (label offsets, inertia,
 * build/buy/outsource/market strategies), pipelines, links/flows, evolve, trends, notes,
 * numbered annotations, and accelerator/deaccelerator forces.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_MEMBERS, MAX_NODES, MAX_SVG_SIDE, checkCount } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError, type PreparedSource } from "../../types";

/** Sourcing strategy decorator on a Wardley component. */
export type WardleyStrategy = "build" | "buy" | "outsource" | "market";

/** An evolution stage column on the X-axis. */
export interface WardleyStage {
  label: string;
  sublabel?: string;
  boundary: number;
}

/** A component, anchor, or pipeline member on the map. */
export interface WardleyNode {
  name: string;
  kind: "anchor" | "component" | "pipeline-member";
  visibility: number;
  evolution: number;
  labelOffset?: [number, number];
  inertia: boolean;
  strategy?: WardleyStrategy;
  evolveTo?: number;
  trendTo?: [number, number];
  pipelineParent?: string;
}

/** A pipeline grouping child components sharing the parent's visibility. */
export interface WardleyPipeline {
  parent: string;
  visibility: number;
  members: string[];
}

/** A dependency or flow link between two nodes. */
export interface WardleyLink {
  from: string;
  to: string;
  kind: "dep" | "dashed" | "flow" | "rflow" | "biflow";
  label: string;
}

/** A numbered annotation point. */
export interface WardleyAnnotation {
  num: number;
  visibility: number;
  evolution: number;
  text: string;
}

/** A positioned note box on the map. */
export interface WardleyNote {
  text: string;
  visibility: number;
  evolution: number;
}

/** An accelerator or deaccelerator force marker. */
export interface WardleyForce {
  kind: "accelerator" | "deaccelerator";
  text: string;
  visibility: number;
  evolution: number;
}

/** A parsed Wardley Map. */
export interface WardleyModel {
  title: string;
  width: number;
  height: number;
  stages: WardleyStage[];
  nodes: WardleyNode[];
  pipelines: WardleyPipeline[];
  links: WardleyLink[];
  notes: WardleyNote[];
  annotationsBox?: [number, number];
  annotations: WardleyAnnotation[];
  forces: WardleyForce[];
}

const DEFAULT_STAGES: WardleyStage[] = [
  { label: "Genesis", boundary: 0.25 },
  { label: "Custom-Built", boundary: 0.5 },
  { label: "Product", sublabel: "+rental", boundary: 0.75 },
  { label: "Commodity", sublabel: "+utility", boundary: 1.0 },
];

const STRATEGIES = new Set<WardleyStrategy>(["build", "buy", "outsource", "market"]);
const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style|linkStyle)\b|:::/i;

function unitCoord(raw: string, what: string): number {
  const num = Number(raw);
  if (!Number.isFinite(num) || num < 0 || num > 1) {
    throw new DiagramInputError(`${what} must be between 0 and 1: ${raw}`);
  }
  return num;
}

function parseName(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.startsWith('"')) {
    if (trimmed.length < 2 || !trimmed.endsWith('"')) {
      throw new DiagramInputError(`Unterminated name quote: ${trimmed}`);
    }
    return clampLabel(trimmed.slice(1, -1));
  }
  return clampLabel(trimmed);
}

/** Find the first position bracket that can satisfy the original coordinate grammar. */
function positionBracket(raw: string, needsComma: boolean): [string, string, string] | undefined {
  let open = -1;
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (ch === "[" && open < 0 && i > 0) open = i;
    else if (ch === "]" && open >= 0) {
      const coordinate = raw.slice(open + 1, i);
      const comma = coordinate.indexOf(",");
      if (
        (needsComma && comma > 0 && comma < coordinate.length - 1) ||
        (!needsComma && coordinate.length > 0)
      ) {
        return [raw.slice(0, open), coordinate, raw.slice(i + 1)];
      }
      open = -1;
    }
  }
  return undefined;
}

function parseDecoratorsAndOffset(tail: string): {
  labelOffset?: [number, number];
  inertia: boolean;
  strategy?: WardleyStrategy;
} {
  let rest = tail.trim();
  let labelOffset: [number, number] | undefined;
  let inertia = false;
  let strategy: WardleyStrategy | undefined;

  while (rest.length > 0) {
    const labelMatch = /^label\s*\[\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*\](.*)$/i.exec(
      rest,
    );
    if (labelMatch) {
      labelOffset = [Number(labelMatch[1]), Number(labelMatch[2])];
      rest = (labelMatch[3] ?? "").trim();
      continue;
    }
    const decMatch = /^\(([^)]+)\)(.*)$/.exec(rest);
    if (decMatch) {
      const dec = (decMatch[1] ?? "").trim().toLowerCase();
      if (dec === "inertia") inertia = true;
      else if (STRATEGIES.has(dec as WardleyStrategy)) strategy = dec as WardleyStrategy;
      else throw new DiagramInputError(`Unknown Wardley decorator: (${dec})`);
      rest = (decMatch[2] ?? "").trim();
      continue;
    }
    throw new DiagramInputError(`Unexpected component suffix: ${rest.slice(0, 40)}`);
  }

  const out: {
    labelOffset?: [number, number];
    inertia: boolean;
    strategy?: WardleyStrategy;
  } = { inertia };
  if (labelOffset !== undefined) out.labelOffset = labelOffset;
  if (strategy !== undefined) out.strategy = strategy;
  return out;
}

function parseStages(spec: string): WardleyStage[] {
  const parts = spec
    .split("->")
    .map((p) => p.trim())
    .filter(Boolean);
  if (parts.length < 2 || parts.length > 12) {
    throw new DiagramInputError("Evolution must declare between 2 and 12 stages");
  }
  const parsed: { label: string; sublabel?: string; boundary?: number }[] = parts.map((part) => {
    let text = part;
    let boundary: number | undefined;
    const atMatch = /^(.*)@(\d+(?:\.\d+)?)$/.exec(text);
    if (atMatch) {
      text = (atMatch[1] ?? "").trim();
      boundary = unitCoord(atMatch[2] ?? "", "Stage boundary");
      if (boundary <= 0) throw new DiagramInputError("Stage boundary must be > 0");
    }
    const slashIdx = text.indexOf("/");
    const stage: { label: string; sublabel?: string; boundary?: number } = {
      label: clampLabel(slashIdx >= 0 ? text.slice(0, slashIdx).trim() : text),
    };
    if (slashIdx >= 0) stage.sublabel = clampLabel(text.slice(slashIdx + 1).trim());
    if (boundary !== undefined) stage.boundary = boundary;
    return stage;
  });

  let prev = 0;
  return parsed.map((stage, i) => {
    const fallback = (i + 1) / parsed.length;
    const boundary = stage.boundary ?? (i === parsed.length - 1 ? 1 : fallback);
    if (boundary <= prev) {
      throw new DiagramInputError("Evolution stage boundaries must increase left to right");
    }
    prev = boundary;
    const out: WardleyStage = { label: stage.label, boundary };
    if (stage.sublabel !== undefined) out.sublabel = stage.sublabel;
    return out;
  });
}

function parseLink(line: string): WardleyLink | undefined {
  let main = line;
  let annotation = "";
  const semi = line.indexOf(";");
  if (semi >= 0) {
    main = line.slice(0, semi).trim();
    annotation = clampLabel(line.slice(semi + 1).trim());
  }

  const labeledFlow = /^(.+?)\+'([^']*)'\s*>\s*(.+)$/.exec(main);
  if (labeledFlow) {
    return {
      from: parseName(labeledFlow[1] ?? ""),
      to: parseName(labeledFlow[3] ?? ""),
      kind: "flow",
      label: clampLabel(labeledFlow[2] ?? annotation),
    };
  }

  const ops: [string, WardleyLink["kind"]][] = [
    ["+<>", "biflow"],
    ["-.->", "dashed"],
    ["-->", "dep"],
    ["+>", "flow"],
    ["+<", "rflow"],
    ["->", "dep"],
  ];
  for (const [op, kind] of ops) {
    const idx = main.indexOf(op);
    if (idx > 0) {
      const left = main.slice(0, idx).trim();
      const right = main.slice(idx + op.length).trim();
      if (left && right) {
        return {
          from: parseName(left),
          to: parseName(right),
          kind,
          label: annotation,
        };
      }
    }
  }
  return undefined;
}

/** Parse `wardley-beta` source (text or prepared lines); throws DiagramInputError on invalid syntax or references. */
export function parseWardley(input: string | PreparedSource): WardleyModel {
  const [, ...body] = linesOf(input, "wardley");
  const model: WardleyModel = {
    title: "",
    width: 1100,
    height: 600,
    stages: DEFAULT_STAGES,
    nodes: [],
    pipelines: [],
    links: [],
    notes: [],
    annotations: [],
    forces: [],
  };

  const nodeByName = new Map<string, WardleyNode>();
  let openPipeline: WardleyPipeline | undefined;
  let itemCount = 0;
  let edgeCount = 0;

  const countNode = (): void => {
    itemCount++;
    checkCount(itemCount, MAX_NODES, "map items");
  };
  const countEdge = (): void => {
    edgeCount++;
    checkCount(edgeCount, MAX_EDGES, "map links");
  };

  const registerNode = (node: WardleyNode): void => {
    if (nodeByName.has(node.name)) {
      throw new DiagramInputError(`Duplicate component or anchor: ${node.name}`);
    }
    nodeByName.set(node.name, node);
    model.nodes.push(node);
    countNode();
  };

  for (const raw of body) {
    const line = raw.trim();
    if (openPipeline) {
      if (line === "}") {
        openPipeline = undefined;
        continue;
      }
      const pm = /^component(\s+)(.*)$/i.exec(line);
      const memberBody = pm?.[2] ?? "";
      const position = pm
        ? (positionBracket(memberBody, false) ??
          positionBracket(`${pm[1]?.slice(-1) ?? ""}${memberBody}`, false))
        : undefined;
      if (!position) throw new DiagramInputError(`Unreadable pipeline line: ${line.slice(0, 40)}`);
      const name = parseName(position[0]);
      const evo = unitCoord(position[1].trim(), "Pipeline evolution");
      const suffix = parseDecoratorsAndOffset(position[2]);
      const member: WardleyNode = {
        name,
        kind: "pipeline-member",
        visibility: openPipeline.visibility,
        evolution: evo,
        inertia: suffix.inertia,
        pipelineParent: openPipeline.parent,
      };
      if (suffix.labelOffset !== undefined) member.labelOffset = suffix.labelOffset;
      if (suffix.strategy !== undefined) member.strategy = suffix.strategy;
      registerNode(member);
      openPipeline.members.push(name);
      checkCount(
        openPipeline.members.length,
        MAX_MEMBERS,
        `members in pipeline ${openPipeline.parent}`,
      );
      continue;
    }

    if (/^(?:accTitle|accDescr)\b/i.test(line)) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    const titleMatch = /^title\s*:?\s*(.+)$/i.exec(line);
    if (titleMatch) {
      model.title = clampLabel((titleMatch[1] ?? "").trim());
      continue;
    }
    const sizeMatch = /^size\s*\[\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*\]$/i.exec(line);
    if (sizeMatch) {
      const w = Number(sizeMatch[1]);
      const h = Number(sizeMatch[2]);
      if (
        !Number.isFinite(w) ||
        !Number.isFinite(h) ||
        w < 300 ||
        h < 240 ||
        w > MAX_SVG_SIDE ||
        h > MAX_SVG_SIDE
      ) {
        throw new DiagramInputError("Wardley map size must be between 300x240 and 16000");
      }
      model.width = w;
      model.height = h;
      continue;
    }
    const evoStagesMatch = /^evolution\s+(.+)$/i.exec(line);
    if (evoStagesMatch) {
      model.stages = parseStages(evoStagesMatch[1] ?? "");
      continue;
    }
    const nodePrefix = /^(anchor|component)(\s+)(.*)$/i.exec(line);
    const nodeBody = nodePrefix?.[3] ?? "";
    const nodePosition = nodePrefix
      ? (positionBracket(nodeBody, true) ??
        positionBracket(`${nodePrefix[2]?.slice(-1) ?? ""}${nodeBody}`, true))
      : undefined;
    if (nodePrefix && nodePosition) {
      const kind = (nodePrefix[1] ?? "").toLowerCase() as "anchor" | "component";
      const name = parseName(nodePosition[0]);
      const comma = nodePosition[1].indexOf(",");
      const visibility = unitCoord(nodePosition[1].slice(0, comma).trim(), "Visibility");
      const evolution = unitCoord(nodePosition[1].slice(comma + 1).trim(), "Evolution");
      const suffix = parseDecoratorsAndOffset(nodePosition[2]);
      const node: WardleyNode = {
        name,
        kind,
        visibility,
        evolution,
        inertia: suffix.inertia,
      };
      if (suffix.labelOffset !== undefined) node.labelOffset = suffix.labelOffset;
      if (suffix.strategy !== undefined) node.strategy = suffix.strategy;
      registerNode(node);
      continue;
    }
    const pipeMatch = /^pipeline\s(.+)\{$/i.exec(line);
    if (pipeMatch) {
      const parent = parseName(pipeMatch[1] ?? "");
      const parentNode = nodeByName.get(parent);
      if (!parentNode) throw new DiagramInputError(`Unknown pipeline component: ${parent}`);
      openPipeline = { parent, visibility: parentNode.visibility, members: [] };
      model.pipelines.push(openPipeline);
      continue;
    }
    const evolvePrefix = /^evolve\s/i.exec(line);
    const evolveTail = evolvePrefix ? line.slice(evolvePrefix[0].length) : "";
    let evolveSeparator = evolveTail.length - 1;
    while (evolveSeparator >= 0 && !/\s/.test(evolveTail[evolveSeparator] ?? "")) {
      evolveSeparator--;
    }
    const evolveValue = evolveTail.slice(evolveSeparator + 1);
    if (evolvePrefix && evolveSeparator > 0 && /^\d+(?:\.\d+)?$/.test(evolveValue)) {
      const name = parseName(evolveTail.slice(0, evolveSeparator));
      const target = nodeByName.get(name);
      if (!target) throw new DiagramInputError(`Unknown component in evolve: ${name}`);
      target.evolveTo = unitCoord(evolveValue, "Evolve target");
      countEdge();
      continue;
    }
    const trendMatch = /^(.+?)-\.-\s*\(([^,)]+),([^)]+)\)$/.exec(line);
    if (trendMatch) {
      const name = parseName(trendMatch[1] ?? "");
      const target = nodeByName.get(name);
      if (!target) throw new DiagramInputError(`Unknown component in trend: ${name}`);
      target.trendTo = [
        unitCoord((trendMatch[2] ?? "").trim(), "Trend X"),
        unitCoord((trendMatch[3] ?? "").trim(), "Trend Y"),
      ];
      countEdge();
      continue;
    }
    const noteMatch = /^note\s+"([^"]*)"\s*\[([^,\]]+),([^\]]+)\]$/i.exec(line);
    if (noteMatch) {
      model.notes.push({
        text: clampLabel(noteMatch[1] ?? ""),
        visibility: unitCoord((noteMatch[2] ?? "").trim(), "Note visibility"),
        evolution: unitCoord((noteMatch[3] ?? "").trim(), "Note evolution"),
      });
      countNode();
      continue;
    }
    const annBoxMatch = /^annotations\s*\[([^,\]]+),([^\]]+)\]$/i.exec(line);
    if (annBoxMatch) {
      model.annotationsBox = [
        unitCoord((annBoxMatch[1] ?? "").trim(), "Annotations visibility"),
        unitCoord((annBoxMatch[2] ?? "").trim(), "Annotations evolution"),
      ];
      continue;
    }
    const annMatch = /^annotation\s+(\d+)\s*,\s*\[([^,\]]+),([^\]]+)\]\s+"([^"]*)"$/i.exec(line);
    if (annMatch) {
      model.annotations.push({
        num: Number(annMatch[1]),
        visibility: unitCoord((annMatch[2] ?? "").trim(), "Annotation visibility"),
        evolution: unitCoord((annMatch[3] ?? "").trim(), "Annotation evolution"),
        text: clampLabel(annMatch[4] ?? ""),
      });
      countNode();
      continue;
    }
    const forceMatch = /^(accelerator|deaccelerator)\s+"([^"]*)"\s*\[([^,\]]+),([^\]]+)\]$/i.exec(
      line,
    );
    if (forceMatch) {
      model.forces.push({
        kind: (forceMatch[1] ?? "").toLowerCase() as "accelerator" | "deaccelerator",
        text: clampLabel(forceMatch[2] ?? ""),
        visibility: unitCoord((forceMatch[3] ?? "").trim(), "Force visibility"),
        evolution: unitCoord((forceMatch[4] ?? "").trim(), "Force evolution"),
      });
      countNode();
      continue;
    }

    const link = parseLink(line);
    if (link) {
      if (!nodeByName.has(link.from) || !nodeByName.has(link.to)) {
        throw new DiagramInputError(
          `Link references unknown component: ${link.from} -> ${link.to}`,
        );
      }
      model.links.push(link);
      countEdge();
      continue;
    }

    throw new DiagramInputError(`Unreadable wardley line: ${line.slice(0, 40)}`);
  }

  if (openPipeline) throw new DiagramInputError(`Unclosed pipeline: ${openPipeline.parent}`);
  if (model.nodes.length === 0) {
    throw new DiagramInputError("A Wardley Map needs at least one anchor or component");
  }
  return model;
}
