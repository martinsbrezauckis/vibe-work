/**
 * Architecture diagram parser. Grammar follows Mermaid's architecture-beta page: nested `group`,
 * `service` (optional icon), `junction`, ported edges (`T|B|L|R`, arrows `<`/`>`, `{group}`),
 * `align row|column`, and inline `%%` comments. Styling, `class`, `:::`, links and callbacks are
 * refused.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** Cardinal port side on a service, junction, or group boundary. */
export type Side = "T" | "B" | "L" | "R";

/** A container grouping services, junctions, and child groups. */
export interface ArchGroup {
  id: string;
  icon: string;
  label: string;
  parent?: string | undefined;
}

/** A service node with optional icon, display label, and parent group. */
export interface ArchService {
  id: string;
  icon: string;
  label: string;
  parent?: string | undefined;
}

/** A 4-way edge split junction node. */
export interface ArchJunction {
  id: string;
  parent?: string | undefined;
}

/** A ported edge between two services or junctions. */
export interface ArchEdge {
  from: string;
  fromGroup: boolean;
  fromSide: Side;
  fromArrow: boolean;
  to: string;
  toGroup: boolean;
  toSide: Side;
  toArrow: boolean;
}

/** An explicit row or column alignment directive across two or more nodes. */
export interface ArchAlign {
  axis: "row" | "column";
  members: string[];
}

/** A parsed architecture diagram. */
export interface ArchitectureModel {
  groups: ArchGroup[];
  services: ArchService[];
  junctions: ArchJunction[];
  edges: ArchEdge[];
  aligns: ArchAlign[];
}

const REFUSED = /^(?:style|classDef|class|click|link|linkStyle|callback)\b|:::/i;
const ID = "[A-Za-z_][A-Za-z0-9_-]*";
const GROUP_RE = new RegExp(
  `^group\\s+(${ID})(?:\\(([^)]*)\\))?(?:\\[([^\\]]*)\\])?(?:\\s+in\\s+(${ID}))?$`,
  "i",
);
const SERVICE_RE = new RegExp(
  `^service\\s+(${ID})(?:\\(([^)]*)\\))?(?:\\[([^\\]]*)\\])?(?:\\s+in\\s+(${ID}))?$`,
  "i",
);
const JUNCTION_RE = new RegExp(`^junction\\s+(${ID})(?:\\s+in\\s+(${ID}))?$`, "i");
const EDGE_RE = new RegExp(
  `^(${ID})(\\{group\\})?:([TBLR])\\s*(<)?--(>)?\\s*([TBLR]):(${ID})(\\{group\\})?$`,
  "i",
);

function stripInlineComment(raw: string): string {
  let inQuotes = false;
  let inBracket = false;
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (ch === "\\") {
      i++;
      continue;
    }
    if (ch === '"') inQuotes = !inQuotes;
    else if (!inQuotes && ch === "[") inBracket = true;
    else if (!inQuotes && ch === "]") inBracket = false;
    else if (!inQuotes && !inBracket && ch === "%" && raw[i + 1] === "%") {
      return raw.slice(0, i).trim();
    }
  }
  return raw.trim();
}

function unquoteBracket(raw: string | undefined, fallback: string): string {
  if (raw === undefined) return fallback;
  const trimmed = raw.trim();
  if (trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"')) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

/** Parse `architecture-beta` or `architecture` source; throws DiagramInputError or DiagramLimitError. */
export function parseArchitecture(input: string | PreparedSource): ArchitectureModel {
  const [, ...body] = linesOf(input, "architecture");
  const model: ArchitectureModel = {
    groups: [],
    services: [],
    junctions: [],
    edges: [],
    aligns: [],
  };

  const groupIds = new Map<string, ArchGroup>();
  const nodeParents = new Map<string, string | undefined>();
  const allIds = new Set<string>();
  const checkTotalNodes = (): void => {
    checkCount(
      model.groups.length + model.services.length + model.junctions.length,
      MAX_NODES,
      "architecture components",
    );
  };
  const checkParent = (parent: string | undefined): void => {
    if (parent !== undefined && !groupIds.has(parent)) {
      throw new DiagramInputError(`Unknown parent group: ${parent}`);
    }
  };
  const depthOfGroup = (id: string): number => {
    let cur: string | undefined = id;
    let depth = 0;
    const seen = new Set<string>();
    while (cur !== undefined) {
      if (seen.has(cur)) throw new DiagramInputError(`Cyclic group nesting at ${cur}`);
      seen.add(cur);
      depth++;
      if (depth > MAX_DEPTH) {
        throw new DiagramLimitError(
          `Diagram too large: groups nested more than ${String(MAX_DEPTH)} deep`,
        );
      }
      cur = groupIds.get(cur)?.parent;
    }
    return depth;
  };

  for (const raw of body) {
    const line = stripInlineComment(raw);
    if (line === "") continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    if (/^acc(?:Title|Descr)\b/i.test(line)) continue;

    let m = GROUP_RE.exec(line);
    if (m) {
      const id = m[1] ?? "";
      const icon = (m[2] ?? "").trim();
      const label = unquoteBracket(m[3], id);
      const parent = m[4];
      if (allIds.has(id)) throw new DiagramInputError(`Duplicate component id: ${id}`);
      checkParent(parent);
      const grp: ArchGroup = { id, icon, label, parent };
      groupIds.set(id, grp);
      allIds.add(id);
      model.groups.push(grp);
      depthOfGroup(id);
      checkTotalNodes();
      continue;
    }

    m = SERVICE_RE.exec(line);
    if (m) {
      const id = m[1] ?? "";
      const icon = (m[2] ?? "").trim();
      const label = unquoteBracket(m[3], id);
      const parent = m[4];
      if (allIds.has(id)) throw new DiagramInputError(`Duplicate component id: ${id}`);
      checkParent(parent);
      allIds.add(id);
      nodeParents.set(id, parent);
      model.services.push({ id, icon, label, parent });
      checkTotalNodes();
      continue;
    }

    m = JUNCTION_RE.exec(line);
    if (m) {
      const id = m[1] ?? "";
      const parent = m[2];
      if (allIds.has(id)) throw new DiagramInputError(`Duplicate component id: ${id}`);
      checkParent(parent);
      allIds.add(id);
      nodeParents.set(id, parent);
      model.junctions.push({ id, parent });
      checkTotalNodes();
      continue;
    }

    m = EDGE_RE.exec(line);
    if (m) {
      const from = m[1] ?? "";
      const fromGroup = Boolean(m[2]);
      const fromSide = (m[3] ?? "R").toUpperCase() as Side;
      const fromArrow = Boolean(m[4]);
      const toArrow = Boolean(m[5]);
      const toSide = (m[6] ?? "L").toUpperCase() as Side;
      const to = m[7] ?? "";
      const toGroup = Boolean(m[8]);
      if (!nodeParents.has(from)) {
        throw new DiagramInputError(
          `Edge endpoint must be a declared service or junction: ${from}`,
        );
      }
      if (!nodeParents.has(to)) {
        throw new DiagramInputError(`Edge endpoint must be a declared service or junction: ${to}`);
      }
      if (fromGroup && nodeParents.get(from) === undefined) {
        throw new DiagramInputError(`{group} modifier requires ${from} to belong to a group`);
      }
      if (toGroup && nodeParents.get(to) === undefined) {
        throw new DiagramInputError(`{group} modifier requires ${to} to belong to a group`);
      }
      model.edges.push({
        from,
        fromGroup,
        fromSide,
        fromArrow,
        to,
        toGroup,
        toSide,
        toArrow,
      });
      checkCount(model.edges.length, MAX_EDGES, "edges");
      continue;
    }

    const alignMatch = /^align\s+(row|column)\s+(.+)$/i.exec(line);
    if (alignMatch) {
      const axis = (alignMatch[1] ?? "").toLowerCase() as "row" | "column";
      const members = (alignMatch[2] ?? "").trim().split(/\s+/).filter(Boolean);
      if (members.length < 2) {
        throw new DiagramInputError("align requires at least two members");
      }
      for (const member of members) {
        if (!nodeParents.has(member)) {
          throw new DiagramInputError(
            `align member must be a declared service or junction: ${member}`,
          );
        }
      }
      model.aligns.push({ axis, members });
      continue;
    }

    throw new DiagramInputError(`Unreadable architecture line: ${line.slice(0, 40)}`);
  }

  if (model.services.length + model.junctions.length + model.groups.length === 0) {
    throw new DiagramInputError("An architecture diagram needs components");
  }

  // Validate that declared align ordering does not contradict opposite-port edges between members.
  for (const align of model.aligns) {
    const pos = new Map<string, number>();
    align.members.forEach((id, i) => {
      if (pos.has(id)) throw new DiagramInputError(`Duplicate member in align: ${id}`);
      pos.set(id, i);
    });
    for (const edge of model.edges) {
      const a = pos.get(edge.from);
      const b = pos.get(edge.to);
      if (a === undefined || b === undefined) continue;
      if (align.axis === "row") {
        if (edge.fromSide === "R" && edge.toSide === "L" && a > b) {
          throw new DiagramInputError(
            `align row order conflicts with ${edge.from}:R -- L:${edge.to}`,
          );
        }
        if (edge.fromSide === "L" && edge.toSide === "R" && a < b) {
          throw new DiagramInputError(
            `align row order conflicts with ${edge.from}:L -- R:${edge.to}`,
          );
        }
      } else {
        if (edge.fromSide === "B" && edge.toSide === "T" && a > b) {
          throw new DiagramInputError(
            `align column order conflicts with ${edge.from}:B -- T:${edge.to}`,
          );
        }
        if (edge.fromSide === "T" && edge.toSide === "B" && a < b) {
          throw new DiagramInputError(
            `align column order conflicts with ${edge.from}:T -- B:${edge.to}`,
          );
        }
      }
    }
  }

  return model;
}
