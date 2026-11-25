/**
 * State diagram parser. Grammar follows Mermaid's state diagram page: `[*]` start and end, `state`
 * with descriptions, composites with `--` concurrent regions, fork/join/choice, labelled
 * transitions and left/right notes (one line or `end note`). CSS class attachment is refused.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** Kinds of state: ordinary, start, end, fork, join and choice. */
export type StateKind = "state" | "start" | "end" | "fork" | "join" | "choice";
/** A state with label, description lines, kind, containing composite and concurrent regions. */
export interface StateNode {
  id: string;
  label: string;
  description: string[];
  kind: StateKind;
  /** Composite that holds this state; undefined at the top level. */
  parent?: string | undefined;
  /** For composites: the children of each concurrent region, in order of appearance. */
  regions: string[][];
}
/** A transition between two states with an optional label and the composite that holds it. */
export interface StateTransition {
  from: string;
  to: string;
  label: string;
  /** Composite whose region holds the transition; undefined at the top level. */
  scope?: string | undefined;
}
/** A note beside a state. */
export interface StateNote {
  side: "left" | "right";
  target: string;
  text: string;
}
/** A parsed state diagram. */
export interface StateModel {
  direction: "TB" | "BT" | "LR" | "RL";
  states: StateNode[];
  transitions: StateTransition[];
  notes: StateNote[];
}

interface Scope {
  /** Composite id; undefined for the top level. */
  id: string | undefined;
  region: number;
}

const IGNORED = /^(?:hide\s+empty\s+description|scale\s+\d+|accTitle\b|accDescr\b)/i;
const REFUSED = /^(?:class|classDef|cssClass|click|link|callback)\b|:::/;

/** Parse `stateDiagram` source (text or prepared lines); throws DiagramInputError on anything unreadable or unclosed. */
export function parseState(input: string | PreparedSource): StateModel {
  const [, ...body] = linesOf(input, "state");
  const model: StateModel = { direction: "TB", states: [], transitions: [], notes: [] };
  const byId = new Map<string, StateNode>();
  const scopes: Scope[] = [{ id: undefined, region: 0 }];
  const top = (): Scope => scopes[scopes.length - 1] as Scope;
  let noteOpen: { side: "left" | "right"; target: string; lines: string[] } | undefined;

  const place = (node: StateNode): void => {
    const scope = top();
    node.parent = scope.id;
    if (scope.id !== undefined) byId.get(scope.id)?.regions[scope.region]?.push(node.id);
  };
  const declare = (id: string, kind: StateKind = "state"): StateNode => {
    let node = byId.get(id);
    if (!node) {
      node = { id, label: id, description: [], kind, regions: [] };
      byId.set(id, node);
      model.states.push(node);
      checkCount(model.states.length, MAX_NODES, "states");
      place(node);
    } else if (kind !== "state") node.kind = kind;
    return node;
  };
  /** `[*]` is one start or end per region of the current scope. */
  const endpoint = (raw: string, role: "start" | "end"): string => {
    if (raw !== "[*]") return declare(raw).id;
    const scope = top();
    return declare(
      `${role === "start" ? "__start" : "__end"}_${scope.id ?? "root"}_${String(scope.region)}`,
      role,
    ).id;
  };

  for (const raw of body) {
    const line = raw.trim();
    let match: RegExpExecArray | null;
    if (noteOpen) {
      if (/^end\s+note$/i.test(line)) {
        model.notes.push({
          side: noteOpen.side,
          target: noteOpen.target,
          text: noteOpen.lines.join("\n"),
        });
        noteOpen = undefined;
      } else noteOpen.lines.push(line);
      continue;
    }
    if (IGNORED.test(line)) continue;
    if (REFUSED.test(line)) throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    if ((match = /^direction\s+(TB|TD|BT|LR|RL)$/i.exec(line))) {
      const d = (match[1] ?? "").toUpperCase();
      model.direction = d === "TD" ? "TB" : (d as StateModel["direction"]);
    } else if ((match = /^note\s+(left|right)\s+of\s+(\S+)\s*:\s*(.*)$/i.exec(line))) {
      model.notes.push({
        side: (match[1] ?? "").toLowerCase() as "left" | "right",
        target: declare(match[2] ?? "").id,
        text: (match[3] ?? "").trim(),
      });
    } else if ((match = /^note\s+(left|right)\s+of\s+(\S+)$/i.exec(line))) {
      noteOpen = {
        side: (match[1] ?? "").toLowerCase() as "left" | "right",
        target: declare(match[2] ?? "").id,
        lines: [],
      };
    } else if (
      (match =
        /^state\s+(?:"([^"]*)"\s+as\s+(\S+?)|(\S+?))\s*(\{|<<\s*(fork|join|choice)\s*>>)?$/i.exec(
          line,
        ))
    ) {
      const id = match[2] ?? match[3] ?? "";
      const kind = (match[5]?.toLowerCase() ?? "state") as StateKind;
      const node = declare(id, kind);
      if (match[1] !== undefined) node.label = match[1];
      if (match[4] === "{") {
        if (scopes.length > MAX_DEPTH)
          throw new DiagramLimitError(
            `Diagram too large: states nested more than ${String(MAX_DEPTH)} deep`,
          );
        if (node.regions.length === 0) node.regions.push([]);
        scopes.push({ id, region: 0 });
      }
    } else if (line === "--") {
      const scope = top();
      if (scope.id === undefined)
        throw new DiagramInputError("`--` belongs inside a composite state");
      const node = byId.get(scope.id);
      node?.regions.push([]);
      scope.region = (node?.regions.length ?? 1) - 1;
    } else if (line === "}") {
      if (scopes.length === 1) throw new DiagramInputError("`}` without an open state");
      scopes.pop();
    } else if ((match = /^(\S+)\s*-->\s*(\S+)\s*(?::\s*(.*))?$/.exec(line))) {
      const from = endpoint(match[1] ?? "", "start");
      const to = endpoint(match[2] ?? "", "end");
      model.transitions.push({ from, to, label: (match[3] ?? "").trim(), scope: top().id });
      checkCount(model.transitions.length, MAX_EDGES, "transitions");
    } else if ((match = /^(\S+)\s*:\s*(.+)$/.exec(line))) {
      declare(match[1] ?? "").description.push((match[2] ?? "").trim());
    } else if (/^[^\s:{}"<>-]+$/.test(line)) {
      declare(line);
    } else {
      throw new DiagramInputError(`Unreadable state line: ${line.slice(0, 40)}`);
    }
  }
  if (noteOpen) throw new DiagramInputError("Unclosed note");
  if (scopes.length > 1) throw new DiagramInputError("Unclosed composite state");
  if (model.states.length === 0) throw new DiagramInputError("A state diagram needs states");
  return model;
}
