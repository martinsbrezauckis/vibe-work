/**
 * Entity-relationship parser. Grammar follows Mermaid's ER page: relationships with crow's-foot
 * cardinality symbols or words, identifying (`--`, `to`) and non-identifying (`..`, `optionally to`)
 * lines, entity blocks with `type name PK, FK, UK "comment"` attributes, aliases `id["Label"]`, and
 * `direction`. CSS class attachment is refused.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_MEMBERS, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** Crow's-foot cardinality at one end of a relationship. */
export type Cardinality = "zero-one" | "one" | "zero-many" | "one-many";
/** An entity attribute with type, name, keys and comment. */
export interface ErAttribute {
  type: string;
  name: string;
  keys: ("PK" | "FK" | "UK")[];
  comment: string;
}
/** An entity with its display label and attributes. */
export interface ErEntity {
  id: string;
  label: string;
  attributes: ErAttribute[];
}
/** A relationship with cardinalities at both ends, line style and label. */
export interface ErRelation {
  from: string;
  to: string;
  /** Cardinality at the `from` end and at the `to` end. */
  fromCard: Cardinality;
  toCard: Cardinality;
  identifying: boolean;
  label: string;
}
/** A parsed ER diagram. */
export interface ErModel {
  direction: "TB" | "BT" | "LR" | "RL";
  entities: ErEntity[];
  relations: ErRelation[];
}

const LEFT_SYMBOLS: Record<string, Cardinality> = {
  "|o": "zero-one",
  "||": "one",
  "}o": "zero-many",
  "}|": "one-many",
};
const RIGHT_SYMBOLS: Record<string, Cardinality> = {
  "o|": "zero-one",
  "||": "one",
  "o{": "zero-many",
  "|{": "one-many",
};
const WORDS: [string, Cardinality][] = [
  ["zero or one", "zero-one"],
  ["one or zero", "zero-one"],
  ["zero or more", "zero-many"],
  ["zero or many", "zero-many"],
  ["many(0)", "zero-many"],
  ["0+", "zero-many"],
  ["one or more", "one-many"],
  ["one or many", "one-many"],
  ["many(1)", "one-many"],
  ["1+", "one-many"],
  ["only one", "one"],
  ["1", "one"],
];
const WORD_PATTERN = WORDS.map(([word]) => word.replace(/[()+]/g, "\\$&")).join("|");
const SYMBOL_SPEC = /^([|}][o|])(--|\.\.)([o|][|{])$/;
const WORD_SPEC = new RegExp(`^(${WORD_PATTERN})\\s+(to|optionally to)\\s+(${WORD_PATTERN})$`, "i");
const NAME = '(?:"[^"]+"|[\\p{L}\\p{N}_][\\p{L}\\p{N}_.-]*)';
const RELATION = new RegExp(
  `^(${NAME}(?:\\[[^\\]]*\\])?)\\s+(.+?)\\s+(${NAME}(?:\\[[^\\]]*\\])?)\\s*:\\s*(.*)$`,
  "u",
);
const REFUSED = /^(?:class|classDef|cssClass|click|link|callback)\b|:::/;

const unquote = (raw: string): string => (raw.startsWith('"') ? raw.slice(1, -1) : raw);
const word = (text: string): Cardinality | undefined =>
  WORDS.find(([w]) => w === text.toLowerCase())?.[1];

/** Parse `erDiagram` source (text or prepared lines); throws DiagramInputError on anything unreadable or unclosed. */
export function parseEr(input: string | PreparedSource): ErModel {
  const [, ...body] = linesOf(input, "er");
  const model: ErModel = { direction: "TB", entities: [], relations: [] };
  const byId = new Map<string, ErEntity>();
  let open: ErEntity | undefined;

  /** `NAME` or `NAME[Alias]` / `NAME["Alias"]`. */
  const entity = (raw: string): ErEntity => {
    const m = /^(.+?)(?:\[\s*"?([^\]"]*)"?\s*\])?$/.exec(raw.trim());
    const id = unquote((m?.[1] ?? raw).trim());
    let found = byId.get(id);
    if (!found) {
      found = { id, label: id, attributes: [] };
      byId.set(id, found);
      model.entities.push(found);
      checkCount(model.entities.length, MAX_NODES, "entities");
    }
    if (m?.[2]) found.label = m[2];
    return found;
  };

  for (const raw of body) {
    const line = raw.trim();
    let match: RegExpExecArray | null;
    if (open) {
      if (line === "}") {
        open = undefined;
        continue;
      }
      match = /^(\S+)\s+(\S+)(?:\s+((?:PK|FK|UK)(?:\s*,\s*(?:PK|FK|UK))*))?(?:\s+"([^"]*)")?$/.exec(
        line,
      );
      if (!match) throw new DiagramInputError(`Unreadable attribute: ${line.slice(0, 40)}`);
      open.attributes.push({
        type: match[1] ?? "",
        name: match[2] ?? "",
        keys: (match[3]?.split(",").map((k) => k.trim()) ?? []) as ErAttribute["keys"],
        comment: match[4] ?? "",
      });
      checkCount(open.attributes.length, MAX_MEMBERS, "attributes in one entity");
      continue;
    }
    if (REFUSED.test(line)) throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    if ((match = /^direction\s+(TB|TD|BT|LR|RL)$/i.exec(line))) {
      const d = (match[1] ?? "").toUpperCase();
      model.direction = d === "TD" ? "TB" : (d as ErModel["direction"]);
    } else if ((match = /^(\S+?(?:\[[^\]]*\])?)\s*\{$/.exec(line))) {
      open = entity(match[1] ?? "");
    } else if ((match = RELATION.exec(line))) {
      const spec = match[2] ?? "";
      const symbols = SYMBOL_SPEC.exec(spec);
      const words = symbols ? undefined : WORD_SPEC.exec(spec);
      const fromCard = symbols
        ? LEFT_SYMBOLS[symbols[1] ?? ""]
        : words
          ? word(words[1] ?? "")
          : undefined;
      const toCard = symbols
        ? RIGHT_SYMBOLS[symbols[3] ?? ""]
        : words
          ? word(words[3] ?? "")
          : undefined;
      if (!fromCard || !toCard)
        throw new DiagramInputError(`Unreadable relationship: ${spec.slice(0, 20)}`);
      const identifying = symbols ? symbols[2] === "--" : (words?.[2] ?? "").toLowerCase() === "to";
      model.relations.push({
        from: entity(match[1] ?? "").id,
        to: entity(match[3] ?? "").id,
        fromCard,
        toCard,
        identifying,
        label: unquote((match[4] ?? "").trim()),
      });
      checkCount(model.relations.length, MAX_EDGES, "relations");
    } else if (/^(?:"[^"]+"|[\p{L}\p{N}_][\p{L}\p{N}_.-]*)(?:\[[^\]]*\])?$/u.test(line)) {
      entity(line);
    } else {
      throw new DiagramInputError(`Unreadable ER line: ${line.slice(0, 40)}`);
    }
  }
  if (open) throw new DiagramInputError("Unclosed entity body");
  if (model.entities.length === 0) throw new DiagramInputError("An ER diagram needs entities");
  return model;
}
