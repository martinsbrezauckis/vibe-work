/**
 * Event Modeling diagram parser. Grammar follows Mermaid's eventmodeling page: compact (`tf`/`rf`)
 * and relaxed (`timeframe`/`resetframe`) frame lines, entity types (`ui`, `pcr`/`processor`,
 * `cmd`/`command`, `rmo`/`readmodel`, `evt`/`event`), namespace swimlanes (`Ns.Entity`), inline
 * and `[[DataBlock]]` data with optional backtick data types, and inferred/explicit `->>` edges.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_MEMBERS, MAX_NODES, checkCount } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError, type PreparedSource } from "../../types";

/** Normalized Event Modeling entity type codes. */
export type EmEntityType = "ui" | "pcr" | "cmd" | "rmo" | "evt";

/** A swimlane row ordered by first appearance in the source. */
export interface EmLane {
  id: string;
  label: string;
  namespace?: string;
}

/** A single time frame (`tf`) or reset frame (`rf`) on the timeline. */
export interface EmFrame {
  num: string;
  isReset: boolean;
  laneId: string;
  type: EmEntityType;
  name: string;
  displayName: string;
  namespace?: string;
  dataRef?: string;
  inlineData?: string;
  inlineDataType?: string;
  explicitRefs: string[];
}

/** A relation edge between two frame numbers. */
export interface EmEdge {
  from: string;
  to: string;
  inferred: boolean;
}

/** A named `data` block referenced via `[[id]]`. */
export interface EmDataBlock {
  dataType?: string;
  fields: string[];
}

/** A parsed Event Modeling diagram. */
export interface EventModelingModel {
  lanes: EmLane[];
  frames: EmFrame[];
  edges: EmEdge[];
  dataBlocks: Map<string, EmDataBlock>;
}

const TYPE_ALIASES: Record<string, EmEntityType> = {
  ui: "ui",
  pcr: "pcr",
  processor: "pcr",
  cmd: "cmd",
  command: "cmd",
  rmo: "rmo",
  readmodel: "rmo",
  evt: "evt",
  event: "evt",
};

const TYPE_LABEL: Record<EmEntityType, string> = {
  ui: "UI",
  pcr: "Processor",
  cmd: "Command",
  rmo: "Read Model",
  evt: "Event",
};

const DATA_TYPES = new Set(["json", "jsobj", "figma", "salt", "uri", "md", "html", "text"]);
const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style|linkStyle)\b|:::/i;

function laneInfo(
  type: EmEntityType,
  namespace: string | undefined,
): { id: string; label: string } {
  if (namespace) {
    return {
      id: `ns:${namespace}:${type}`,
      label: `${namespace} · ${TYPE_LABEL[type]}`,
    };
  }
  if (type === "ui" || type === "pcr") {
    return { id: "default:ui-pcr", label: "UI / Automation" };
  }
  if (type === "cmd" || type === "rmo") {
    return { id: "default:cmd-rmo", label: "Command / Read Model" };
  }
  return { id: "default:evt", label: "Events" };
}

function consumeBraced(text: string, openIdx: number): { content: string; nextIdx: number } {
  let depth = 0;
  let quote: '"' | "'" | undefined;
  for (let i = openIdx; i < text.length; i++) {
    const ch = text[i] ?? "";
    if (quote) {
      if (ch === "\\" && i + 1 < text.length) {
        i++;
        continue;
      }
      if (ch === quote) quote = undefined;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (ch === "{") {
      depth++;
    } else if (ch === "}") {
      depth--;
      if (depth === 0) {
        return { content: text.slice(openIdx + 1, i).trim(), nextIdx: i + 1 };
      }
    }
  }
  throw new DiagramInputError("Unterminated inline data `{` block");
}

function parseFrameRemainder(rest: string): {
  dataRef?: string;
  inlineData?: string;
  inlineDataType?: string;
  explicitRefs: string[];
} {
  let cursor = 0;
  let dataRef: string | undefined;
  let inlineData: string | undefined;
  let inlineDataType: string | undefined;
  const explicitRefs: string[] = [];

  const skipWs = (): void => {
    while (cursor < rest.length && /\s/.test(rest[cursor] ?? "")) cursor++;
  };

  skipWs();
  if (rest.startsWith("[[", cursor)) {
    const end = rest.indexOf("]]", cursor + 2);
    if (end < 0) throw new DiagramInputError("Unterminated `[[` data block reference");
    const ref = rest.slice(cursor + 2, end).trim();
    if (!ref) throw new DiagramInputError("Empty data block reference `[[ ]]`");
    dataRef = ref;
    cursor = end + 2;
  }

  skipWs();
  if (rest[cursor] === "`" || rest[cursor] === "{") {
    if (dataRef !== undefined) {
      throw new DiagramInputError("Frame cannot carry both `[[id]]` and inline data");
    }
    if (rest[cursor] === "`") {
      const endTick = rest.indexOf("`", cursor + 1);
      if (endTick < 0) throw new DiagramInputError("Unterminated data type backtick");
      const dtype = rest
        .slice(cursor + 1, endTick)
        .trim()
        .toLowerCase();
      if (!DATA_TYPES.has(dtype)) throw new DiagramInputError(`Unknown data type: ${dtype}`);
      inlineDataType = dtype;
      cursor = endTick + 1;
      skipWs();
    }
    if (rest[cursor] !== "{") {
      throw new DiagramInputError("Expected `{` after data type tag");
    }
    const parsed = consumeBraced(rest, cursor);
    inlineData = clampLabel(parsed.content);
    cursor = parsed.nextIdx;
  }

  while (cursor < rest.length) {
    skipWs();
    if (cursor >= rest.length) break;
    if (!rest.startsWith("->>", cursor)) {
      throw new DiagramInputError(
        `Unexpected token in frame line: ${rest.slice(cursor, cursor + 30)}`,
      );
    }
    cursor += 3;
    skipWs();
    const match = /^(\S+)/.exec(rest.slice(cursor));
    if (!match?.[1]) throw new DiagramInputError("Expected frame number after `->>`");
    explicitRefs.push(match[1]);
    cursor += match[1].length;
  }

  const out: {
    dataRef?: string;
    inlineData?: string;
    inlineDataType?: string;
    explicitRefs: string[];
  } = { explicitRefs };
  if (dataRef !== undefined) out.dataRef = dataRef;
  if (inlineData !== undefined) out.inlineData = inlineData;
  if (inlineDataType !== undefined) out.inlineDataType = inlineDataType;
  return out;
}

function parseEntityAndRest(rawTail: string): { entity: string; rest: string } {
  const trimmed = rawTail.trim();
  if (trimmed.startsWith('"')) {
    const end = trimmed.indexOf('"', 1);
    if (end < 0) throw new DiagramInputError("Unterminated entity name quote");
    return {
      entity: trimmed.slice(1, end),
      rest: trimmed.slice(end + 1),
    };
  }
  const match = /^([^\s{[\]]+)(.*)$/.exec(trimmed);
  if (!match?.[1]) throw new DiagramInputError("Missing entity identifier");
  return { entity: match[1], rest: match[2] ?? "" };
}

/** Parse `eventmodeling` source (text or prepared lines); throws DiagramInputError on syntax or reference errors. */
export function parseEventModeling(input: string | PreparedSource): EventModelingModel {
  const [, ...body] = linesOf(input, "eventmodeling");
  const model: EventModelingModel = {
    lanes: [],
    frames: [],
    edges: [],
    dataBlocks: new Map(),
  };
  const laneById = new Map<string, EmLane>();
  const frameByNum = new Map<string, EmFrame>();
  let openData: { id: string; dataType?: string; depth: number; fields: string[] } | undefined;

  for (const raw of body) {
    const line = raw.trim();
    if (openData) {
      for (const ch of line) {
        if (ch === "{") openData.depth++;
        else if (ch === "}") openData.depth--;
      }
      if (openData.depth <= 0) {
        const tail = line.replace(/\}\s*$/, "").trim();
        if (tail) openData.fields.push(clampLabel(tail));
        checkCount(openData.fields.length, MAX_MEMBERS, `fields in data block ${openData.id}`);
        const block: EmDataBlock = { fields: openData.fields };
        if (openData.dataType !== undefined) block.dataType = openData.dataType;
        model.dataBlocks.set(openData.id, block);
        openData = undefined;
      } else if (line) {
        openData.fields.push(clampLabel(line));
        checkCount(openData.fields.length, MAX_MEMBERS, `fields in data block ${openData.id}`);
      }
      continue;
    }

    if (/^(?:accTitle|accDescr)\b/i.test(line)) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    const dataMatch = /^data\s+(\S+)\s*(?:`([^`]+)`\s*)?\{(.*)$/i.exec(line);
    if (dataMatch) {
      const id = dataMatch[1] ?? "";
      const dtype = dataMatch[2]?.trim().toLowerCase();
      if (dtype !== undefined && !DATA_TYPES.has(dtype)) {
        throw new DiagramInputError(`Unknown data type: ${dtype}`);
      }
      if (model.dataBlocks.has(id)) {
        throw new DiagramInputError(`Duplicate data block: ${id}`);
      }
      checkCount(model.dataBlocks.size + 1, MAX_NODES, "data blocks");
      const tail = (dataMatch[3] ?? "").trim();
      let depth = 1;
      for (const ch of tail) {
        if (ch === "{") depth++;
        else if (ch === "}") depth--;
      }
      if (depth <= 0) {
        const inner = tail.replace(/\}\s*$/, "").trim();
        const block: EmDataBlock = { fields: inner ? [clampLabel(inner)] : [] };
        if (dtype !== undefined) block.dataType = dtype;
        model.dataBlocks.set(id, block);
      } else {
        openData = {
          id,
          depth,
          fields: tail ? [clampLabel(tail)] : [],
        };
        if (dtype !== undefined) openData.dataType = dtype;
      }
      continue;
    }

    const frameMatch = /^(tf|timeframe|rf|resetframe)\s+(\S+)\s+(\S+)\s+(.+)$/i.exec(line);
    if (frameMatch) {
      const kw = (frameMatch[1] ?? "").toLowerCase();
      const num = frameMatch[2] ?? "";
      const typeToken = (frameMatch[3] ?? "").toLowerCase();
      const entityType = TYPE_ALIASES[typeToken];
      if (!entityType) {
        throw new DiagramInputError(`Unknown Event Modeling entity type: ${typeToken}`);
      }
      if (frameByNum.has(num)) {
        throw new DiagramInputError(`Duplicate frame number: ${num}`);
      }
      const { entity, rest } = parseEntityAndRest(frameMatch[4] ?? "");
      const dotIdx = entity.indexOf(".");
      const hasNs = dotIdx > 0 && dotIdx < entity.length - 1;
      const namespace = hasNs ? entity.slice(0, dotIdx) : undefined;
      const info = laneInfo(entityType, namespace);
      if (!laneById.has(info.id)) {
        const lane: EmLane = { id: info.id, label: info.label };
        if (namespace !== undefined) lane.namespace = namespace;
        laneById.set(info.id, lane);
        model.lanes.push(lane);
        checkCount(model.lanes.length, MAX_NODES, "swimlanes");
      }

      const parsed = parseFrameRemainder(rest);
      const frame: EmFrame = {
        num,
        isReset: kw === "rf" || kw === "resetframe",
        laneId: info.id,
        type: entityType,
        name: clampLabel(entity),
        displayName: clampLabel(hasNs ? entity.slice(dotIdx + 1) : entity),
        explicitRefs: parsed.explicitRefs,
      };
      if (namespace !== undefined) frame.namespace = namespace;
      if (parsed.dataRef !== undefined) frame.dataRef = parsed.dataRef;
      if (parsed.inlineData !== undefined) frame.inlineData = parsed.inlineData;
      if (parsed.inlineDataType !== undefined) frame.inlineDataType = parsed.inlineDataType;

      frameByNum.set(num, frame);
      model.frames.push(frame);
      checkCount(model.frames.length, MAX_NODES, "frames");
      continue;
    }

    throw new DiagramInputError(`Unreadable eventmodeling line: ${line.slice(0, 40)}`);
  }

  if (openData) throw new DiagramInputError(`Unclosed data block: ${openData.id}`);
  if (model.frames.length === 0) {
    throw new DiagramInputError("An Event Modeling diagram needs at least one frame");
  }

  const seenEdges = new Set<string>();
  const addEdge = (from: string, to: string, inferred: boolean): void => {
    const key = `${from}\u0000${to}`;
    if (seenEdges.has(key)) return;
    seenEdges.add(key);
    model.edges.push({ from, to, inferred });
    checkCount(model.edges.length, MAX_EDGES, "relations");
  };

  model.frames.forEach((frame, i) => {
    if (frame.dataRef !== undefined && !model.dataBlocks.has(frame.dataRef)) {
      throw new DiagramInputError(`Unknown data block reference: ${frame.dataRef}`);
    }
    const prev = i > 0 ? model.frames[i - 1] : undefined;
    if (!frame.isReset && prev) {
      addEdge(prev.num, frame.num, true);
    }
    for (const ref of frame.explicitRefs) {
      if (!frameByNum.has(ref)) {
        throw new DiagramInputError(`Unknown frame reference: ->> ${ref}`);
      }
      if (ref !== frame.num) addEdge(ref, frame.num, false);
    }
  });

  return model;
}
