/**
 * Class diagram parser. Grammar follows Mermaid's class diagram page: `class` with `{}` bodies,
 * `Name : member` lines, labels `Name["Label"]`, generics `Name~T~`, stereotypes, namespaces,
 * the fourteen relationship tokens with cardinalities and labels, and notes. Callbacks, links and
 * CSS class attachment are refused: nothing here may become interactive or styled by the source.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_MEMBERS, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** End decoration of a class relationship. */
export type Head = "inherit" | "compose" | "aggregate" | "arrow";
/** One attribute or method with visibility and classifiers. */
export interface ClassMember {
  visibility: "+" | "-" | "#" | "~" | "";
  /** Member text without visibility and classifier; generics already written as `<T>`. */
  text: string;
  method: boolean;
  abstract: boolean;
  static: boolean;
}
/** A class with label, generic parameter, stereotype, members and namespace. */
export interface ClassNode {
  id: string;
  label: string;
  generic?: string;
  stereotype?: string;
  members: ClassMember[];
  namespace?: string;
}
/** A relationship with decorations at both ends, line style, label and cardinalities. */
export interface ClassRelation {
  from: string;
  to: string;
  /** Marker at the `from` end and at the `to` end. */
  fromHead?: Head;
  toHead?: Head;
  dashed: boolean;
  label: string;
  fromCard: string;
  toCard: string;
}
/** A note, attached to a class or floating. */
export interface ClassNote {
  text: string;
  /** Class the note is attached to; unattached notes float. */
  target?: string;
}
/** A parsed class diagram. */
export interface ClassModel {
  direction: "TB" | "BT" | "LR" | "RL";
  classes: ClassNode[];
  relations: ClassRelation[];
  namespaces: { name: string; classes: string[] }[];
  notes: ClassNote[];
}

const RELATIONS: [string, { fromHead?: Head; toHead?: Head; dashed: boolean }][] = [
  ["<|--", { fromHead: "inherit", dashed: false }],
  ["--|>", { toHead: "inherit", dashed: false }],
  ["<|..", { fromHead: "inherit", dashed: true }],
  ["..|>", { toHead: "inherit", dashed: true }],
  ["*--", { fromHead: "compose", dashed: false }],
  ["--*", { toHead: "compose", dashed: false }],
  ["o--", { fromHead: "aggregate", dashed: false }],
  ["--o", { toHead: "aggregate", dashed: false }],
  ["<--", { fromHead: "arrow", dashed: false }],
  ["-->", { toHead: "arrow", dashed: false }],
  ["<..", { fromHead: "arrow", dashed: true }],
  ["..>", { toHead: "arrow", dashed: true }],
  ["--", { dashed: false }],
  ["..", { dashed: true }],
];
const REL_PATTERN = RELATIONS.map(([token]) => token.replace(/[*|.]/g, "\\$&")).join("|");
const RELATION = new RegExp(
  `^([^\\s"]+?)\\s*(?:"([^"]*)"\\s*)?(${REL_PATTERN})\\s*(?:"([^"]*)"\\s*)?([^\\s"]+?)\\s*(?::\\s*(.*))?$`,
);
const CLASS_REF = /^(?:`([^`]+)`|([^\s~{}"`:]+))(?:~([^~]+)~)?(?:\["([^"]*)"\])?$/;
/** A class reference as written after `class`: plain, generic `~T~`, backticked, or with a `["label"]`. */
const CLASS_WORD = '(?:[^\\s"{]|\\["[^"]*"\\])+?';
const CLASS_OPEN = new RegExp(`^class\\s+(${CLASS_WORD})\\s*\\{$`);
const CLASS_DECL = new RegExp(`^class\\s+(${CLASS_WORD})(?:\\s+<<\\s*(.+?)\\s*>>)?$`);
const REFUSED = /^(?:callback|link|click|cssClass|style|classDef)\b/i;

/** Mermaid writes generics as `~T~` in members; show them as `<T>`. */
const showGenerics = (text: string): string => text.replace(/~([^~]*)~/g, "<$1>");

function parseMember(raw: string): ClassMember {
  let text = raw.trim();
  const visibility = /^[+\-#~]/.test(text) ? (text[0] as ClassMember["visibility"]) : "";
  if (visibility) text = text.slice(1).trim();
  const method = text.includes("(");
  let abstract = false;
  let isStatic = false;
  if (method) {
    const close = text.indexOf(")");
    if (close >= 0) {
      const mark = /^\s*([*$])/.exec(text.slice(close + 1))?.[1];
      if (mark) {
        abstract = mark === "*";
        isStatic = mark === "$";
        text = `${text.slice(0, close + 1)}${text.slice(close + 1).replace(/^\s*[*$]/, "")}`;
      }
    }
  } else if (/[*$]$/.test(text)) {
    abstract = text.endsWith("*");
    isStatic = text.endsWith("$");
    text = text.slice(0, -1).trimEnd();
  }
  return { visibility, text: showGenerics(text), method, abstract, static: isStatic };
}

/** Parse `classDiagram` source (text or prepared lines); throws DiagramInputError on anything unreadable or unclosed. */
export function parseClass(input: string | PreparedSource): ClassModel {
  const [, ...body] = linesOf(input, "class");
  const model: ClassModel = {
    direction: "TB",
    classes: [],
    relations: [],
    namespaces: [],
    notes: [],
  };
  const byId = new Map<string, ClassNode>();
  let current: ClassNode | undefined;
  let namespace: { name: string; classes: string[] } | undefined;

  const ref = (raw: string): ClassNode => {
    const m = CLASS_REF.exec(raw.trim());
    if (!m) throw new DiagramInputError(`Unreadable class name: ${raw.slice(0, 40)}`);
    const id = m[1] ?? m[2] ?? "";
    let node = byId.get(id);
    if (!node) {
      node = { id, label: id, members: [] };
      byId.set(id, node);
      model.classes.push(node);
      checkCount(model.classes.length, MAX_NODES, "classes");
      if (namespace) {
        node.namespace = namespace.name;
        namespace.classes.push(id);
      }
    }
    if (m[3] !== undefined) node.generic = m[3];
    if (m[4] !== undefined) node.label = m[4];
    return node;
  };
  const addMember = (node: ClassNode, text: string): void => {
    node.members.push(parseMember(text));
    checkCount(node.members.length, MAX_MEMBERS, "members in one class");
  };

  for (const raw of body) {
    const line = raw.trim();
    let match: RegExpExecArray | null;
    if (current) {
      if (line === "}") {
        current = undefined;
      } else if ((match = /^<<\s*(.+?)\s*>>$/.exec(line))) {
        current.stereotype = match[1] ?? "";
      } else if (line.startsWith("<<")) {
        throw new DiagramInputError("Unterminated stereotype");
      } else addMember(current, line);
      continue;
    }
    if (REFUSED.test(line)) throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    if ((match = /^direction\s+(TB|TD|BT|LR|RL)$/i.exec(line))) {
      const d = (match[1] ?? "").toUpperCase();
      model.direction = d === "TD" ? "TB" : (d as ClassModel["direction"]);
    } else if ((match = /^namespace\s+(\S+)\s*\{$/i.exec(line))) {
      if (namespace) throw new DiagramInputError("Namespaces do not nest");
      namespace = { name: match[1] ?? "", classes: [] };
      model.namespaces.push(namespace);
      checkCount(model.namespaces.length, MAX_NODES, "namespaces");
    } else if (line === "}" && namespace) {
      namespace = undefined;
    } else if ((match = CLASS_OPEN.exec(line))) {
      current = ref(match[1] ?? "");
    } else if ((match = CLASS_DECL.exec(line))) {
      const node = ref(match[1] ?? "");
      if (match[2]) node.stereotype = match[2];
    } else if ((match = /^<<\s*(.+?)\s*>>\s+(\S+)$/.exec(line))) {
      ref(match[2] ?? "").stereotype = match[1] ?? "";
    } else if ((match = /^note\s+for\s+(\S+)\s+"([^"]*)"$/i.exec(line))) {
      model.notes.push({ text: match[2] ?? "", target: ref(match[1] ?? "").id });
    } else if ((match = /^note\s+"([^"]*)"$/i.exec(line))) {
      model.notes.push({ text: match[1] ?? "" });
    } else if (/^note\b/i.test(line)) {
      throw new DiagramInputError("Unreadable note");
    } else if ((match = RELATION.exec(line))) {
      const kind = RELATIONS.find(([token]) => token === match?.[3])?.[1];
      if (!kind) throw new DiagramInputError("Unknown relationship");
      model.relations.push({
        from: ref(match[1] ?? "").id,
        to: ref(match[5] ?? "").id,
        ...kind,
        label: (match[6] ?? "").trim(),
        fromCard: match[2] ?? "",
        toCard: match[4] ?? "",
      });
      checkCount(model.relations.length, MAX_EDGES, "relations");
    } else if ((match = /^(\S+?)\s*:\s*(.+)$/.exec(line))) {
      addMember(ref(match[1] ?? ""), match[2] ?? "");
    } else {
      throw new DiagramInputError(`Unreadable class line: ${line.slice(0, 40)}`);
    }
  }
  if (current) throw new DiagramInputError("Unclosed class body");
  if (namespace) throw new DiagramInputError("Unclosed namespace");
  if (model.classes.length === 0) throw new DiagramInputError("A class diagram needs classes");
  return model;
}
