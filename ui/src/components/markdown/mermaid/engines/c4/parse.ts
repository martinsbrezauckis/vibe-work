/**
 * C4 diagram parser. Grammar follows Mermaid's C4 page across all five chart types (`C4Context`,
 * `C4Container`, `C4Component`, `C4Dynamic`, `C4Deployment`): Person/System/Container/Component
 * (+ Db/Queue/_Ext), nested Boundaries & Deployment Nodes (Node, Node_L, Node_R), positional and
 * `$named` parameters, Rel/BiRel/Rel_U/D/L/R/Rel_Back/RelIndex, UpdateLayoutConfig, and
 * UpdateRelStyle label offsets. Refuses `$link`, `Lay_*`, custom tags, styling and callbacks.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** The five supported C4 diagram header types. */
export type C4Variant = "C4Context" | "C4Container" | "C4Component" | "C4Dynamic" | "C4Deployment";

/** Visual shape family for a C4 leaf element. */
export type C4ShapeKind = "person" | "box" | "db" | "queue";

/** Directional hint on a relationship statement. */
export type C4RelDir = "U" | "D" | "L" | "R" | "none";

/** A leaf C4 element (Person, System, Container, Component). */
export interface C4Element {
  kind: "element";
  id: string;
  role: "person" | "system" | "container" | "component";
  shape: C4ShapeKind;
  external: boolean;
  label: string;
  techn: string;
  descr: string;
}

/** A boundary or deployment node scope containing nested elements and child scopes. */
export interface C4Scope {
  kind: "scope";
  id: string;
  scopeType: "boundary" | "node";
  align: "center" | "left" | "right";
  label: string;
  typeLabel: string;
  descr: string;
  children: C4Item[];
}

/** An item inside a C4 scope: either a leaf element or a nested boundary/node scope. */
export type C4Item = C4Element | C4Scope;

/** A relationship edge between two C4 elements or scopes. */
export interface C4Relation {
  from: string;
  to: string;
  label: string;
  techn: string;
  descr: string;
  dir: C4RelDir;
  fromArrow: boolean;
  toArrow: boolean;
  sequence?: number | undefined;
}

/** A parsed C4 diagram across all five variants. */
export interface C4Model {
  variant: C4Variant;
  title: string;
  root: { items: C4Item[] };
  relations: C4Relation[];
  shapeInRow: number;
  boundaryInRow: number;
  relOffsets: Map<string, { x: number; y: number }>;
}

interface ElementSpec {
  role: C4Element["role"];
  shape: C4ShapeKind;
  external: boolean;
  hasTechn: boolean;
}

const ELEMENT_SPECS: Record<string, ElementSpec> = {
  person: { role: "person", shape: "person", external: false, hasTechn: false },
  person_ext: { role: "person", shape: "person", external: true, hasTechn: false },
  system: { role: "system", shape: "box", external: false, hasTechn: false },
  systemdb: { role: "system", shape: "db", external: false, hasTechn: false },
  systemqueue: { role: "system", shape: "queue", external: false, hasTechn: false },
  system_ext: { role: "system", shape: "box", external: true, hasTechn: false },
  systemdb_ext: { role: "system", shape: "db", external: true, hasTechn: false },
  systemqueue_ext: { role: "system", shape: "queue", external: true, hasTechn: false },
  container: { role: "container", shape: "box", external: false, hasTechn: true },
  containerdb: { role: "container", shape: "db", external: false, hasTechn: true },
  containerqueue: { role: "container", shape: "queue", external: false, hasTechn: true },
  container_ext: { role: "container", shape: "box", external: true, hasTechn: true },
  containerdb_ext: { role: "container", shape: "db", external: true, hasTechn: true },
  containerqueue_ext: { role: "container", shape: "queue", external: true, hasTechn: true },
  component: { role: "component", shape: "box", external: false, hasTechn: true },
  componentdb: { role: "component", shape: "db", external: false, hasTechn: true },
  componentqueue: { role: "component", shape: "queue", external: false, hasTechn: true },
  component_ext: { role: "component", shape: "box", external: true, hasTechn: true },
  componentdb_ext: { role: "component", shape: "db", external: true, hasTechn: true },
  componentqueue_ext: { role: "component", shape: "queue", external: true, hasTechn: true },
};

const SCOPE_DEFAULTS: Record<
  string,
  {
    scopeType: "boundary" | "node";
    align: "center" | "left" | "right";
    defaultType: string;
    hasDescr: boolean;
  }
> = {
  boundary: { scopeType: "boundary", align: "center", defaultType: "Boundary", hasDescr: false },
  enterprise_boundary: {
    scopeType: "boundary",
    align: "center",
    defaultType: "Enterprise",
    hasDescr: false,
  },
  system_boundary: {
    scopeType: "boundary",
    align: "center",
    defaultType: "System",
    hasDescr: false,
  },
  container_boundary: {
    scopeType: "boundary",
    align: "center",
    defaultType: "Container",
    hasDescr: false,
  },
  deployment_node: {
    scopeType: "node",
    align: "center",
    defaultType: "Deployment Node",
    hasDescr: true,
  },
  node: { scopeType: "node", align: "center", defaultType: "Node", hasDescr: true },
  node_l: { scopeType: "node", align: "left", defaultType: "Node", hasDescr: true },
  node_r: { scopeType: "node", align: "right", defaultType: "Node", hasDescr: true },
};

const REL_SPECS: Record<
  string,
  { dir: C4RelDir; fromArrow: boolean; toArrow: boolean; indexed: boolean }
> = {
  rel: { dir: "none", fromArrow: false, toArrow: true, indexed: false },
  birel: { dir: "none", fromArrow: true, toArrow: true, indexed: false },
  rel_u: { dir: "U", fromArrow: false, toArrow: true, indexed: false },
  rel_up: { dir: "U", fromArrow: false, toArrow: true, indexed: false },
  rel_d: { dir: "D", fromArrow: false, toArrow: true, indexed: false },
  rel_down: { dir: "D", fromArrow: false, toArrow: true, indexed: false },
  rel_l: { dir: "L", fromArrow: false, toArrow: true, indexed: false },
  rel_left: { dir: "L", fromArrow: false, toArrow: true, indexed: false },
  rel_r: { dir: "R", fromArrow: false, toArrow: true, indexed: false },
  rel_right: { dir: "R", fromArrow: false, toArrow: true, indexed: false },
  rel_back: { dir: "none", fromArrow: true, toArrow: false, indexed: false },
  relindex: { dir: "none", fromArrow: false, toArrow: true, indexed: true },
};

const REFUSED_CMD =
  /^(?:lay_u|lay_up|lay_d|lay_down|lay_l|lay_left|lay_r|lay_right|addelementtag|addreltag|style|classdef|class|click|link|linkstyle|callback)$/i;
const VARIANT_MAP: Record<string, C4Variant> = {
  c4context: "C4Context",
  c4container: "C4Container",
  c4component: "C4Component",
  c4dynamic: "C4Dynamic",
  c4deployment: "C4Deployment",
};

function stripInlineComment(raw: string): string {
  let inQuotes = false;
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (ch === "\\") {
      i++;
      continue;
    }
    if (ch === '"') inQuotes = !inQuotes;
    else if (!inQuotes && ch === "%" && raw[i + 1] === "%") return raw.slice(0, i).trim();
  }
  return raw.trim();
}

function unquote(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed.startsWith('"')) return trimmed;
  let out = "";
  let closed = false;
  for (let i = 1; i < trimmed.length; i++) {
    const ch = trimmed[i];
    if (ch === "\\") {
      const next = trimmed[i + 1];
      if (next === undefined) throw new DiagramInputError("Unterminated quoted string in C4");
      out += next;
      i++;
      continue;
    }
    if (ch === '"') {
      closed = true;
      if (trimmed.slice(i + 1).trim() !== "") {
        throw new DiagramInputError("Unexpected text after closing quote in C4");
      }
      break;
    }
    out += ch ?? "";
  }
  if (!closed) throw new DiagramInputError("Unterminated quoted string in C4");
  return out;
}

function splitCallArgs(rawArgs: string): { named?: string; value: string }[] {
  if (rawArgs.trim() === "") return [];
  const parts: string[] = [];
  let inQuotes = false;
  let start = 0;
  for (let i = 0; i < rawArgs.length; i++) {
    const ch = rawArgs[i];
    if (ch === "\\") {
      i++;
      continue;
    }
    if (ch === '"') inQuotes = !inQuotes;
    else if (!inQuotes && ch === ",") {
      parts.push(rawArgs.slice(start, i));
      start = i + 1;
    }
  }
  if (inQuotes) throw new DiagramInputError("Unterminated quote in C4 argument");
  parts.push(rawArgs.slice(start));

  return parts.map((part) => {
    const trimmed = part.trim();
    const namedMatch = /^\$([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/.exec(trimmed);
    if (namedMatch) {
      return { named: (namedMatch[1] ?? "").toLowerCase(), value: unquote(namedMatch[2] ?? "") };
    }
    return { value: unquote(trimmed) };
  });
}

function bindParams(rawArgs: string, order: readonly string[]): Record<string, string> {
  const args = splitCallArgs(rawArgs);
  const bound: Record<string, string> = {};
  let pos = 0;
  let seenNamed = false;
  for (const arg of args) {
    if (arg.named !== undefined) {
      seenNamed = true;
      bound[arg.named] = arg.value;
    } else {
      if (seenNamed) {
        throw new DiagramInputError("Positional argument cannot follow a $named argument in C4");
      }
      const key = order[pos++];
      if (key) bound[key.toLowerCase()] = arg.value;
    }
  }
  if (bound.link) {
    throw new DiagramInputError("Links ($link) are not drawn in C4 diagrams");
  }
  return bound;
}

function parseCall(line: string): { fn: string; args: string; openBrace: boolean } | undefined {
  const fnMatch = /^([A-Za-z_][A-Za-z0-9_]*)\s*\(/.exec(line);
  if (!fnMatch) return undefined;
  const fn = fnMatch[1] ?? "";
  let i = fnMatch[0].length;
  let depth = 1;
  let inQuotes = false;
  while (i < line.length && depth > 0) {
    const ch = line[i];
    if (ch === "\\") {
      i += 2;
      continue;
    }
    if (ch === '"') inQuotes = !inQuotes;
    else if (!inQuotes) {
      if (ch === "(") depth++;
      else if (ch === ")") depth--;
    }
    i++;
  }
  if (depth !== 0) throw new DiagramInputError(`Unclosed call ${fn}(`);
  const args = line.slice(fnMatch[0].length, i - 1);
  const rest = line.slice(i).trim();
  if (rest !== "" && rest !== "{") {
    throw new DiagramInputError(`Unexpected trailing text after ${fn}(...): ${rest.slice(0, 24)}`);
  }
  return { fn, args, openBrace: rest === "{" };
}

function clampOffset(raw: string | undefined): number {
  if (raw === undefined || raw.trim() === "") return 0;
  const val = Number(raw);
  if (!Number.isFinite(val)) throw new DiagramInputError(`Invalid C4 offset value: ${raw}`);
  return Math.max(-300, Math.min(300, val));
}

/** Parse any of the five C4 diagram variants; throws DiagramInputError or DiagramLimitError. */
export function parseC4(input: string | PreparedSource): C4Model {
  const [header, ...body] = linesOf(input, "c4");
  const variant = VARIANT_MAP[(header ?? "").trim().toLowerCase()];
  if (!variant) throw new DiagramInputError(`Unknown C4 header: ${header ?? ""}`);

  const model: C4Model = {
    variant,
    title: "",
    root: { items: [] },
    relations: [],
    shapeInRow: 4,
    boundaryInRow: 2,
    relOffsets: new Map(),
  };

  const stack: { items: C4Item[] }[] = [model.root];
  const allIds = new Set<string>();
  let totalNodes = 0;
  let dynamicSeq = 0;

  const pushItem = (item: C4Item): void => {
    if (allIds.has(item.id)) throw new DiagramInputError(`Duplicate C4 alias: ${item.id}`);
    allIds.add(item.id);
    totalNodes++;
    checkCount(totalNodes, MAX_NODES, "C4 elements and boundaries");
    stack[stack.length - 1]?.items.push(item);
  };

  for (const raw of body) {
    const line = stripInlineComment(raw);
    if (line === "") continue;
    if (
      /^(?:style|classDef|class|click|link|linkStyle|callback)\b/i.test(line) ||
      line.includes(":::")
    ) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    if (/^acc(?:Title|Descr)\b/i.test(line)) continue;

    if (line === "}") {
      if (stack.length === 1) throw new DiagramInputError("`}` without an open C4 boundary");
      stack.pop();
      continue;
    }

    const titleMatch = /^title(?:\s*:)?\s+(.+)$/i.exec(line);
    if (titleMatch) {
      model.title = unquote(titleMatch[1] ?? "");
      continue;
    }

    const call = parseCall(line);
    if (!call) throw new DiagramInputError(`Unreadable C4 line: ${line.slice(0, 40)}`);

    const key = call.fn.toLowerCase();
    if (REFUSED_CMD.test(key)) {
      throw new DiagramInputError(`Unsupported C4 statement: ${call.fn}`);
    }

    const elemSpec = ELEMENT_SPECS[key];
    if (elemSpec) {
      if (call.openBrace) throw new DiagramInputError(`${call.fn} cannot open a { } block`);
      const order = elemSpec.hasTechn
        ? ["alias", "label", "techn", "descr", "sprite", "tags", "link"]
        : ["alias", "label", "descr", "sprite", "tags", "link"];
      const p = bindParams(call.args, order);
      const id = (p.alias ?? "").trim();
      const label = p.label ?? "";
      if (!id || !label) throw new DiagramInputError(`${call.fn} requires alias and label`);
      pushItem({
        kind: "element",
        id,
        role: elemSpec.role,
        shape: elemSpec.shape,
        external: elemSpec.external,
        label,
        techn: p.techn ?? "",
        descr: p.descr ?? "",
      });
      continue;
    }

    const scopeSpec = SCOPE_DEFAULTS[key];
    if (scopeSpec) {
      const order = scopeSpec.hasDescr
        ? ["alias", "label", "type", "descr", "sprite", "tags", "link"]
        : key === "boundary"
          ? ["alias", "label", "type", "tags", "link"]
          : ["alias", "label", "tags", "link"];
      const p = bindParams(call.args, order);
      const id = (p.alias ?? "").trim();
      const label = p.label ?? "";
      if (!id || !label) throw new DiagramInputError(`${call.fn} requires alias and label`);
      const scope: C4Scope = {
        kind: "scope",
        id,
        scopeType: scopeSpec.scopeType,
        align: scopeSpec.align,
        label,
        typeLabel: p.type && p.type.trim() !== "" ? p.type : scopeSpec.defaultType,
        descr: p.descr ?? "",
        children: [],
      };
      pushItem(scope);
      if (call.openBrace) {
        if (stack.length > MAX_DEPTH) {
          throw new DiagramLimitError(
            `Diagram too large: C4 boundaries nested more than ${String(MAX_DEPTH)} deep`,
          );
        }
        stack.push({ items: scope.children });
      }
      continue;
    }

    const relSpec = REL_SPECS[key];
    if (relSpec) {
      if (call.openBrace) throw new DiagramInputError(`${call.fn} cannot open a { } block`);
      const order = relSpec.indexed
        ? ["index", "from", "to", "label", "techn", "descr", "tags", "link"]
        : ["from", "to", "label", "techn", "descr", "sprite", "tags", "link"];
      const p = bindParams(call.args, order);
      const from = (p.from ?? "").trim();
      const to = (p.to ?? "").trim();
      if (!from || !to) throw new DiagramInputError(`${call.fn} requires from and to aliases`);
      model.relations.push({
        from,
        to,
        label: p.label ?? "",
        techn: p.techn ?? "",
        descr: p.descr ?? "",
        dir: relSpec.dir,
        fromArrow: relSpec.fromArrow,
        toArrow: relSpec.toArrow,
        sequence: relSpec.indexed ? ++dynamicSeq : undefined,
      });
      checkCount(model.relations.length, MAX_EDGES, "C4 relationships");
      continue;
    }

    if (key === "updatelayoutconfig") {
      const p = bindParams(call.args, ["c4ShapeInRow", "c4BoundaryInRow"]);
      if (p.c4shapeinrow !== undefined && p.c4shapeinrow !== "") {
        const val = Number(p.c4shapeinrow);
        if (!Number.isSafeInteger(val) || val < 1) {
          throw new DiagramInputError("c4ShapeInRow must be a positive integer");
        }
        model.shapeInRow = Math.min(20, val);
      }
      if (p.c4boundaryinrow !== undefined && p.c4boundaryinrow !== "") {
        const val = Number(p.c4boundaryinrow);
        if (!Number.isSafeInteger(val) || val < 1) {
          throw new DiagramInputError("c4BoundaryInRow must be a positive integer");
        }
        model.boundaryInRow = Math.min(20, val);
      }
      continue;
    }

    if (key === "updateelementstyle") {
      // Parsed for compatibility; custom colors/sprites are ignored so drawings stay token-themed.
      const p = bindParams(call.args, [
        "elementName",
        "bgColor",
        "fontColor",
        "borderColor",
        "shadowing",
        "shape",
        "sprite",
        "techn",
        "legendText",
        "legendSprite",
      ]);
      if (!p.elementname) throw new DiagramInputError("UpdateElementStyle requires elementName");
      continue;
    }

    if (key === "updaterelstyle") {
      const p = bindParams(call.args, [
        "from",
        "to",
        "textColor",
        "lineColor",
        "offsetX",
        "offsetY",
      ]);
      const from = (p.from ?? "").trim();
      const to = (p.to ?? "").trim();
      if (!from || !to) throw new DiagramInputError("UpdateRelStyle requires from and to");
      model.relOffsets.set(`${from}->${to}`, {
        x: clampOffset(p.offsetx),
        y: clampOffset(p.offsety),
      });
      continue;
    }

    throw new DiagramInputError(`Unreadable C4 call: ${call.fn}`);
  }

  if (stack.length > 1) throw new DiagramInputError("Unclosed C4 boundary");
  if (totalNodes === 0) throw new DiagramInputError("A C4 diagram needs elements or boundaries");

  for (const rel of model.relations) {
    if (!allIds.has(rel.from))
      throw new DiagramInputError(`Unknown C4 alias in relationship: ${rel.from}`);
    if (!allIds.has(rel.to))
      throw new DiagramInputError(`Unknown C4 alias in relationship: ${rel.to}`);
  }
  return model;
}
