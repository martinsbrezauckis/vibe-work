/**
 * Requirement diagram parser. Grammar follows Mermaid's requirement diagram page: six SysML
 * requirement types, element blocks (`type`, `docref`), `direction`, quoted/unquoted values,
 * forward/reverse relationships (`contains`, `copies`, `derives`, `satisfies`, `verifies`,
 * `refines`, `traces`), and inline `%%` comments. Styling, `class`, `:::`, links and callbacks
 * are refused.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** The six SysML requirement classifications supported by Mermaid. */
export type RequirementKind =
  | "requirement"
  | "functionalRequirement"
  | "interfaceRequirement"
  | "performanceRequirement"
  | "physicalRequirement"
  | "designConstraint";

/** Normalized risk severity levels. */
export type RiskLevel = "Low" | "Medium" | "High";

/** Normalized verification methods. */
export type VerifyMethod = "Analysis" | "Inspection" | "Test" | "Demonstration";

/** Relationship kinds connecting requirements and elements. */
export type RelationKind =
  "contains" | "copies" | "derives" | "satisfies" | "verifies" | "refines" | "traces";

/** A requirement or element node in the diagram. */
export type RequirementNode =
  | {
      kind: "requirement";
      id: string;
      reqType: RequirementKind;
      reqId: string;
      text: string;
      risk?: RiskLevel;
      verifymethod?: VerifyMethod;
    }
  | {
      kind: "element";
      id: string;
      elemType: string;
      docRef: string;
    };

/** A directed relationship from `from` to `to`. */
export interface RequirementRelation {
  from: string;
  to: string;
  kind: RelationKind;
}

/** A parsed requirement diagram. */
export interface RequirementModel {
  direction: "TB" | "BT" | "LR" | "RL";
  nodes: RequirementNode[];
  relations: RequirementRelation[];
}

const REQ_KINDS: Record<string, RequirementKind> = {
  requirement: "requirement",
  functionalrequirement: "functionalRequirement",
  interfacerequirement: "interfaceRequirement",
  performancerequirement: "performanceRequirement",
  physicalrequirement: "physicalRequirement",
  designconstraint: "designConstraint",
};

const RISK_LEVELS: Record<string, RiskLevel> = {
  low: "Low",
  medium: "Medium",
  high: "High",
};

const VERIFY_METHODS: Record<string, VerifyMethod> = {
  analysis: "Analysis",
  inspection: "Inspection",
  test: "Test",
  demonstration: "Demonstration",
};

const RELATION_KINDS: Record<string, RelationKind> = {
  contains: "contains",
  copies: "copies",
  derives: "derives",
  satisfies: "satisfies",
  verifies: "verifies",
  refines: "refines",
  traces: "traces",
};

const REFUSED = /^(?:style|classDef|class|click|link|linkStyle|callback)\b|:::/i;
const NAME_TOKEN = '"(?:[^"\\\\]|\\\\.)*"|[^\\s{}<>:]+';
const BLOCK_OPEN = new RegExp(`^([A-Za-z]+)\\s+(${NAME_TOKEN})\\s*\\{(\\s*\\})?$`);
const FWD_REL = new RegExp(`^(${NAME_TOKEN})\\s+-\\s*([A-Za-z]+)\\s*->\\s*(${NAME_TOKEN})$`);
const REV_REL = new RegExp(`^(${NAME_TOKEN})\\s+<-\\s*([A-Za-z]+)\\s*-\\s*(${NAME_TOKEN})$`);

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

function parseValue(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed.startsWith('"')) return trimmed;
  let out = "";
  let closed = false;
  for (let i = 1; i < trimmed.length; i++) {
    const ch = trimmed[i];
    if (ch === "\\") {
      const next = trimmed[i + 1];
      if (next === undefined) throw new DiagramInputError("Unterminated quoted value");
      out += next;
      i++;
      continue;
    }
    if (ch === '"') {
      closed = true;
      if (trimmed.slice(i + 1).trim() !== "") {
        throw new DiagramInputError("Unexpected text after closing quote");
      }
      break;
    }
    out += ch ?? "";
  }
  if (!closed) throw new DiagramInputError("Unterminated quoted value");
  return out;
}

/** Parse `requirementDiagram` source; throws DiagramInputError or DiagramLimitError on problems. */
export function parseRequirement(input: string | PreparedSource): RequirementModel {
  const [, ...body] = linesOf(input, "requirement");
  const model: RequirementModel = { direction: "TB", nodes: [], relations: [] };
  const byId = new Map<string, RequirementNode>();
  let open: RequirementNode | undefined;

  for (const raw of body) {
    const line = stripInlineComment(raw);
    if (line === "") continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    if (open) {
      if (line === "}") {
        open = undefined;
        continue;
      }
      const fieldMatch = /^([A-Za-z]+)\s*:\s*(.*)$/.exec(line);
      if (!fieldMatch) {
        throw new DiagramInputError(`Unreadable requirement field: ${line.slice(0, 40)}`);
      }
      const key = (fieldMatch[1] ?? "").toLowerCase();
      const val = parseValue(fieldMatch[2] ?? "");
      if (open.kind === "requirement") {
        if (key === "id") open.reqId = val;
        else if (key === "text") open.text = val;
        else if (key === "risk") {
          const level = RISK_LEVELS[val.toLowerCase()];
          if (!level) throw new DiagramInputError(`Unknown risk level: ${val}`);
          open.risk = level;
        } else if (key === "verifymethod") {
          const method = VERIFY_METHODS[val.toLowerCase()];
          if (!method) throw new DiagramInputError(`Unknown verification method: ${val}`);
          open.verifymethod = method;
        } else {
          throw new DiagramInputError(`Unknown requirement property: ${key}`);
        }
      } else {
        if (key === "type") open.elemType = val;
        else if (key === "docref") open.docRef = val;
        else throw new DiagramInputError(`Unknown element property: ${key}`);
      }
      continue;
    }

    if (/^acc(?:Title|Descr)\b/i.test(line)) continue;
    const dirMatch = /^direction\s+(TB|TD|BT|LR|RL)$/i.exec(line);
    if (dirMatch) {
      const dir = (dirMatch[1] ?? "").toUpperCase();
      model.direction = dir === "TD" ? "TB" : (dir as RequirementModel["direction"]);
      continue;
    }

    const blockMatch = BLOCK_OPEN.exec(line);
    if (blockMatch) {
      const kw = (blockMatch[1] ?? "").toLowerCase();
      const id = parseValue(blockMatch[2] ?? "");
      if (id === "") throw new DiagramInputError("Requirement or element name cannot be empty");
      if (byId.has(id)) throw new DiagramInputError(`Duplicate requirement node: ${id}`);
      let node: RequirementNode;
      if (kw === "element") {
        node = { kind: "element", id, elemType: "", docRef: "" };
      } else {
        const reqType = REQ_KINDS[kw];
        if (!reqType) throw new DiagramInputError(`Unknown block keyword: ${blockMatch[1] ?? ""}`);
        node = { kind: "requirement", id, reqType, reqId: "", text: "" };
      }
      byId.set(id, node);
      model.nodes.push(node);
      checkCount(model.nodes.length, MAX_NODES, "nodes");
      if (blockMatch[3] === undefined) open = node;
      continue;
    }

    const fwd = FWD_REL.exec(line);
    const rev = fwd ? undefined : REV_REL.exec(line);
    const relMatch = fwd ?? rev;
    if (relMatch) {
      const relKey = (relMatch[2] ?? "").toLowerCase();
      const kind = RELATION_KINDS[relKey];
      if (!kind) throw new DiagramInputError(`Unknown relationship type: ${relMatch[2] ?? ""}`);
      const left = parseValue(relMatch[1] ?? "");
      const right = parseValue(relMatch[3] ?? "");
      model.relations.push({
        from: fwd ? left : right,
        to: fwd ? right : left,
        kind,
      });
      checkCount(model.relations.length, MAX_EDGES, "relationships");
      continue;
    }

    throw new DiagramInputError(`Unreadable requirement line: ${line.slice(0, 40)}`);
  }

  if (open) throw new DiagramInputError("Unclosed requirement block");
  if (model.nodes.length === 0) {
    throw new DiagramInputError("A requirement diagram needs requirements or elements");
  }
  for (const rel of model.relations) {
    if (!byId.has(rel.from)) {
      throw new DiagramInputError(`Unknown requirement or element: ${rel.from}`);
    }
    if (!byId.has(rel.to)) {
      throw new DiagramInputError(`Unknown requirement or element: ${rel.to}`);
    }
  }
  return model;
}
