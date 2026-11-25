/**
 * Use case diagram parser. Grammar follows Mermaid's usecase page: `usecase-beta`, `direction`,
 * `actor` variants (`normal`/`hollow`/`awesome`/`icon`, `business`), ellipse/rect use cases,
 * `<<stereotype>>`, one-level `systemBoundary` (`rect`/`package`), strict `json` tables, `note for`,
 * solid/include/extend/generalization relationships with explicit `id@op` and `minlen`.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_MEMBERS, MAX_NODES, checkCount } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError, type PreparedSource } from "../../types";
import {
  autoIdFromTitle,
  decodeUsecaseLabel,
  parseStrictJsonTable,
  type JsonTableRow,
} from "./json";
import { parseSimpleMeta, readBoundaryHead, readLabelledAssociation } from "./scan";

/** Layout direction of a use case diagram. */
export type UsecaseDirection = "TB" | "BT" | "LR" | "RL";
/** Actor visual variant. */
export type ActorVariant = "normal" | "hollow" | "awesome" | "icon";

/** An actor, use case, or JSON table element. */
export type UsecaseElement =
  | {
      kind: "actor";
      id: string;
      label: string;
      variant: ActorVariant;
      icon?: string;
      business: boolean;
      stereotype?: string;
      boundaryId?: string;
    }
  | {
      kind: "usecase";
      id: string;
      label: string;
      shape: "ellipse" | "rect";
      business: boolean;
      stereotype?: string;
      boundaryId?: string;
    }
  | {
      kind: "json";
      id: string;
      rows: JsonTableRow[];
    };

/** A one-level-deep system boundary box or package. */
export interface UsecaseBoundary {
  id: string;
  title: string;
  type: "rect" | "package";
  memberIds: string[];
}

/** A note attached to an actor or use case. */
export interface UsecaseNote {
  target: string;
  text: string;
}

/** Relationship marker types. */
export type RelMarker = "none" | "arrow" | "circle" | "cross" | "triangle";

/** A relationship between two elements. */
export interface UsecaseRelation {
  id?: string;
  from: string;
  to: string;
  kind: "assoc" | "include" | "extend" | "generalize";
  startMarker: RelMarker;
  endMarker: RelMarker;
  dashed: boolean;
  minlen: number;
  label: string;
}

/** A parsed use case diagram. */
export interface UsecaseModel {
  direction: UsecaseDirection;
  elements: UsecaseElement[];
  boundaries: UsecaseBoundary[];
  notes: UsecaseNote[];
  relations: UsecaseRelation[];
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style|linkStyle)\b|:::/i;
const ACTOR_META_KEYS = new Set(["type", "icon", "business"]);
const ACTOR_TYPES = new Set(["normal", "hollow", "awesome"]);

function mergeLogicalStatements(body: readonly string[]): string[] {
  const out: string[] = [];
  let i = 0;
  while (i < body.length) {
    let cur = (body[i] ?? "").trim();
    i++;
    if (!cur) continue;
    if (/^accDescr\s*\{$/i.test(cur)) {
      while (i < body.length && (body[i] ?? "").trim() !== "}") i++;
      if (i >= body.length) throw new DiagramInputError("Unclosed accDescr block");
      i++;
      continue;
    }
    // Multiline Markdown string (`"...` without closing `` `" ``)
    const countMd = (s: string): number =>
      (s.match(/`"/g) ?? []).length + (s.match(/"`/g) ?? []).length;
    if (cur.includes('"`') && !cur.includes('`"')) {
      const chunks = [cur];
      while (i < body.length) {
        const next = (body[i] ?? "").trim();
        i++;
        chunks.push(next);
        if (next.includes('`"')) break;
      }
      cur = chunks.join("\n");
    }
    if (cur.includes('"`') && !cur.includes('`"') && countMd(cur) % 2 === 1) {
      throw new DiagramInputError("Unterminated Markdown string");
    }
    // Multiline `@{ ... }` or `json id@{ ... }`
    const atBrace = cur.indexOf("@{");
    if (atBrace >= 0) {
      let depth = 0;
      let inStr = false;
      const scan = (s: string): void => {
        for (let k = 0; k < s.length; k++) {
          const ch = s[k] ?? "";
          if (inStr) {
            if (ch === "\\" && k + 1 < s.length) k++;
            else if (ch === '"') inStr = false;
          } else if (ch === '"') inStr = true;
          else if (ch === "{") depth++;
          else if (ch === "}") depth--;
        }
      };
      scan(cur.slice(atBrace + 1));
      const chunks = [cur];
      while (depth > 0 && i < body.length) {
        const next = body[i] ?? "";
        i++;
        chunks.push(next);
        scan(next);
      }
      cur = chunks.join("\n");
      if (depth > 0) throw new DiagramInputError("Unterminated `@{ ... }` block");
    }
    out.push(cur);
  }
  return out;
}

function extractSuffix(tail: string): {
  meta?: string;
  stereotype?: string;
} {
  let rest = tail.trim();
  let meta: string | undefined;
  let stereotype: string | undefined;
  if (rest.startsWith("@{")) {
    let depth = 0;
    let inStr = false;
    let end = -1;
    for (let i = 1; i < rest.length; i++) {
      const ch = rest[i] ?? "";
      if (inStr) {
        if (ch === "\\" && i + 1 < rest.length) i++;
        else if (ch === '"') inStr = false;
      } else if (ch === '"') inStr = true;
      else if (ch === "{") depth++;
      else if (ch === "}") {
        depth--;
        if (depth === 0) {
          end = i;
          break;
        }
      }
    }
    if (end < 0) throw new DiagramInputError("Unterminated `@{ ... }` metadata");
    meta = rest.slice(2, end);
    rest = rest.slice(end + 1).trim();
  }
  const stereoMatch = /^<<([^>]+)>>(.*)$/.exec(rest);
  if (stereoMatch) {
    stereotype = clampLabel((stereoMatch[1] ?? "").trim());
    rest = (stereoMatch[2] ?? "").trim();
  }
  if (rest !== "") {
    throw new DiagramInputError(`Unexpected trailing tokens: ${rest.slice(0, 40)}`);
  }
  const res: { meta?: string; stereotype?: string } = {};
  if (meta !== undefined) res.meta = meta;
  if (stereotype !== undefined) res.stereotype = stereotype;
  return res;
}

function applyActorMeta(el: Extract<UsecaseElement, { kind: "actor" }>, metaRaw: string): void {
  const meta = parseSimpleMeta(metaRaw);
  for (const key of Object.keys(meta)) {
    if (!ACTOR_META_KEYS.has(key)) {
      throw new DiagramInputError(`Unsupported actor metadata key: ${key}`);
    }
  }
  if (meta.type !== undefined) {
    const t = meta.type.toLowerCase();
    if (!ACTOR_TYPES.has(t)) throw new DiagramInputError(`Unknown actor type: ${meta.type}`);
    el.variant = t as ActorVariant;
  }
  if (meta.icon !== undefined) {
    el.variant = "icon";
    el.icon = clampLabel(meta.icon);
  }
  if (meta.business !== undefined) {
    if (meta.business !== "true" && meta.business !== "false") {
      throw new DiagramInputError(`Invalid business flag: ${meta.business}`);
    }
    el.business = meta.business === "true";
  }
  if (el.icon !== undefined && meta.type && meta.type.toLowerCase() !== "normal") {
    throw new DiagramInputError("Icon actor cannot combine with non-normal type");
  }
  if (el.icon !== undefined && el.business) {
    throw new DiagramInputError("Icon actor cannot be marked business");
  }
  if (el.variant === "awesome" && el.business) {
    throw new DiagramInputError("Awesome actor cannot be marked business");
  }
}

function readDelimited(
  raw: string,
  open: "(" | "[",
  close: ")" | "]",
): { inner: string; rest: string } | undefined {
  const t = raw.trimStart();
  if (!t.startsWith(open)) return undefined;
  let inQuote = false;
  for (let i = 1; i < t.length; i++) {
    const ch = t[i] ?? "";
    if (inQuote) {
      if (ch === "\\" && i + 1 < t.length) {
        i++;
        continue;
      }
      if (ch === '"') inQuote = false;
      continue;
    }
    if (ch === '"') inQuote = true;
    else if (ch === close) {
      return { inner: t.slice(1, i), rest: t.slice(i + 1) };
    }
  }
  throw new DiagramInputError(`Unterminated ${open}...${close} label`);
}

/** Match a quoted label without backtracking; plain labels may contain unpaired backticks. */
function readQuotedLabel(line: string): { inner: string; rest: string } | undefined {
  if (!line.startsWith('"')) return undefined;
  // The suffix must fit on one line, just as the former regex's trailing `(.*)` did.
  let lastBreak = -1;
  for (let i = 1; i < line.length; i++) {
    if (/[\r\n\u2028\u2029]/.test(line[i] ?? "")) lastBreak = i;
  }
  let reachable = true;
  let openBacktick = false;
  for (let i = 1; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (reachable && i > lastBreak) {
        return { inner: line.slice(0, i + 1), rest: line.slice(i + 1) };
      }
      reachable = false;
    } else if (ch === "`") {
      // A reachable backtick can start a segment spanning quotes; any later one can close it.
      if (reachable) openBacktick = true;
      if (openBacktick) reachable = true;
    }
  }
  return undefined;
}

function parseElementDecl(line: string):
  | {
      kind: "actor" | "usecase";
      id: string;
      label: string;
      shape: "ellipse" | "rect";
      meta?: string;
      stereotype?: string;
    }
  | undefined {
  // Actor declaration: `actor ID[(label)|("label")][@{...}][<<stereo>>]`
  const actorMatch = /^actor\s+([A-Za-z0-9_]+)(?![A-Za-z0-9_])(.*)$/.exec(line);
  if (actorMatch) {
    const id = actorMatch[1] ?? "";
    const afterId = (actorMatch[2] ?? "").trimStart();
    const delim = readDelimited(afterId, "(", ")");
    const label = delim ? decodeUsecaseLabel(delim.inner) : id;
    const suffix = extractSuffix(delim ? delim.rest : afterId);
    return { kind: "actor", id, label, shape: "ellipse", ...suffix };
  }
  // Quoted standalone use case: `"..."[@{...}][<<stereo>>]`
  const quotedMatch = readQuotedLabel(line);
  if (quotedMatch) {
    const label = decodeUsecaseLabel(quotedMatch.inner);
    const suffix = extractSuffix(quotedMatch.rest);
    return { kind: "usecase", id: autoIdFromTitle(label), label, shape: "ellipse", ...suffix };
  }
  // Ellipse or Rect use case: `ID(label)[@{...}][<<stereo>>]` or `ID[label][@{...}][<<stereo>>]`
  const idHead = /^([A-Za-z0-9_]+)([[(][\s\S]+)$/.exec(line);
  if (idHead) {
    const id = idHead[1] ?? "";
    const tail = idHead[2] ?? "";
    const isEll = tail.startsWith("(");
    const delim = readDelimited(tail, isEll ? "(" : "[", isEll ? ")" : "]");
    if (delim) {
      const label = decodeUsecaseLabel(delim.inner);
      const suffix = extractSuffix(delim.rest);
      return {
        kind: "usecase",
        id,
        label,
        shape: isEll ? "ellipse" : "rect",
        ...suffix,
      };
    }
  }
  // Bare identifier declaration: `ID`
  if (/^[A-Za-z0-9_]+$/.test(line) && !/^(?:end|direction|systemBoundary|json|note)$/i.test(line)) {
    return { kind: "usecase", id: line, label: line, shape: "ellipse" };
  }
  return undefined;
}

/** Parse `usecase-beta` source (text or prepared lines); throws DiagramInputError on semantic/syntax errors. */
export function parseUsecase(input: string | PreparedSource): UsecaseModel {
  const [, ...rawBody] = linesOf(input, "usecase");
  const statements = mergeLogicalStatements(rawBody);
  const model: UsecaseModel = {
    direction: "TB",
    elements: [],
    boundaries: [],
    notes: [],
    relations: [],
  };
  const elemById = new Map<string, UsecaseElement>();
  const boundaryById = new Map<string, UsecaseBoundary>();
  const edgeIds = new Set<string>();
  let openBoundary: UsecaseBoundary | undefined;

  const ensureUsecase = (id: string): UsecaseElement => {
    let found = elemById.get(id);
    if (!found) {
      found = { kind: "usecase", id, label: id, shape: "ellipse", business: false };
      elemById.set(id, found);
      model.elements.push(found);
      checkCount(model.elements.length, MAX_NODES, "elements");
    }
    return found;
  };

  const registerDecl = (
    decl: NonNullable<ReturnType<typeof parseElementDecl>>,
    boundaryId: string | undefined,
  ): void => {
    let existing = elemById.get(decl.id);
    if (!existing) {
      if (decl.kind === "actor") {
        existing = {
          kind: "actor",
          id: decl.id,
          label: decl.label,
          variant: "normal",
          business: false,
        };
      } else {
        existing = {
          kind: "usecase",
          id: decl.id,
          label: decl.label,
          shape: decl.shape,
          business: false,
        };
      }
      elemById.set(decl.id, existing);
      model.elements.push(existing);
      checkCount(model.elements.length, MAX_NODES, "elements");
    } else if (decl.kind === "actor" && existing.kind === "usecase") {
      // Upgrade an implicitly referenced endpoint to an actor.
      const upgraded: Extract<UsecaseElement, { kind: "actor" }> = {
        kind: "actor",
        id: existing.id,
        label: decl.label,
        variant: "normal",
        business: false,
      };
      if (existing.boundaryId !== undefined) upgraded.boundaryId = existing.boundaryId;
      elemById.set(decl.id, upgraded);
      const idx = model.elements.indexOf(existing);
      if (idx >= 0) model.elements[idx] = upgraded;
      existing = upgraded;
    } else if (existing.kind !== "json") {
      existing.label = decl.label;
      if (existing.kind === "usecase") existing.shape = decl.shape;
    }

    if (boundaryId !== undefined && existing.kind !== "json") {
      if (existing.boundaryId !== undefined && existing.boundaryId !== boundaryId) {
        throw new DiagramInputError(`Element ${decl.id} belongs to multiple boundaries`);
      }
      existing.boundaryId = boundaryId;
      const b = boundaryById.get(boundaryId);
      if (b && !b.memberIds.includes(decl.id)) {
        b.memberIds.push(decl.id);
        checkCount(b.memberIds.length, MAX_MEMBERS, `members in boundary ${boundaryId}`);
      }
    }

    if (decl.stereotype !== undefined && existing.kind !== "json") {
      existing.stereotype = decl.stereotype;
    }
    if (decl.meta !== undefined) {
      if (existing.kind === "actor") {
        applyActorMeta(existing, decl.meta);
      } else if (existing.kind === "usecase") {
        const meta = parseSimpleMeta(decl.meta);
        for (const k of Object.keys(meta)) {
          if (k !== "business")
            throw new DiagramInputError(`Unsupported usecase metadata key: ${k}`);
        }
        if (meta.business !== undefined) {
          if (existing.shape === "rect" && meta.business === "true") {
            throw new DiagramInputError("Rectangular use cases cannot be marked business");
          }
          existing.business = meta.business === "true";
        }
      }
    }
  };

  for (const line of statements) {
    if (/^(?:accTitle|accDescr)\b/i.test(line)) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    if (openBoundary) {
      if (/^end$/i.test(line)) {
        openBoundary = undefined;
        continue;
      }
      if (/^systemBoundary\b/i.test(line)) {
        throw new DiagramInputError("System boundaries cannot nest");
      }
      const decl = parseElementDecl(line);
      if (!decl) {
        throw new DiagramInputError(
          `Only actor and use case declarations belong inside systemBoundary: ${line.slice(0, 40)}`,
        );
      }
      registerDecl(decl, openBoundary.id);
      continue;
    }

    const dirMatch = /^direction\s+(TB|TD|BT|LR|RL)$/i.exec(line);
    if (dirMatch) {
      const d = (dirMatch[1] ?? "TB").toUpperCase();
      model.direction = (d === "TD" ? "TB" : d) as UsecaseDirection;
      continue;
    }

    // `systemBoundary id[title][@{...}]` or `systemBoundary "Title"[@{...}]`
    const sbMatch = readBoundaryHead(line);
    if (sbMatch) {
      const quotedTitle = sbMatch.quotedTitle;
      const id =
        quotedTitle !== undefined
          ? autoIdFromTitle(decodeUsecaseLabel(`"${quotedTitle}"`))
          : (sbMatch.id ?? "");
      const title =
        quotedTitle !== undefined
          ? decodeUsecaseLabel(`"${quotedTitle}"`)
          : sbMatch.squareTitle !== undefined
            ? decodeUsecaseLabel(sbMatch.squareTitle)
            : sbMatch.roundTitle !== undefined
              ? decodeUsecaseLabel(sbMatch.roundTitle)
              : id;
      if (boundaryById.has(id) || elemById.has(id)) {
        throw new DiagramInputError(`Duplicate boundary identifier: ${id}`);
      }
      const suffix = extractSuffix(sbMatch.rest);
      let bType: "rect" | "package" = "rect";
      if (suffix.meta !== undefined) {
        const meta = parseSimpleMeta(suffix.meta);
        if (meta.type !== undefined) {
          if (meta.type !== "rect" && meta.type !== "package") {
            throw new DiagramInputError(`Unknown boundary type: ${meta.type}`);
          }
          bType = meta.type;
        }
      }
      openBoundary = { id, title, type: bType, memberIds: [] };
      boundaryById.set(id, openBoundary);
      model.boundaries.push(openBoundary);
      checkCount(model.boundaries.length, MAX_NODES, "boundaries");
      continue;
    }

    // `json ID@{ ... }`
    const jsonMatch = /^json\s+([A-Za-z0-9_]+)@([\s\S]+)$/.exec(line);
    if (jsonMatch) {
      const id = jsonMatch[1] ?? "";
      const body = (jsonMatch[2] ?? "").trim();
      if (!body.startsWith("{")) {
        throw new DiagramInputError("JSON declaration root must be a strict JSON object `{ ... }`");
      }
      if (elemById.has(id) || boundaryById.has(id)) {
        throw new DiagramInputError(`Duplicate JSON identifier: ${id}`);
      }
      const rows = parseStrictJsonTable(body);
      const node: UsecaseElement = { kind: "json", id, rows };
      elemById.set(id, node);
      model.elements.push(node);
      checkCount(model.elements.length, MAX_NODES, "elements");
      continue;
    }

    // `note for target label`
    const noteMatch = /^note\s+for\s+([A-Za-z0-9_]+)\s+([\s\S]+)$/.exec(line);
    if (noteMatch) {
      const target = noteMatch[1] ?? "";
      if (boundaryById.has(target) || edgeIds.has(target)) {
        throw new DiagramInputError(`Notes cannot target boundary or edge: ${target}`);
      }
      const existing = elemById.get(target);
      if (existing?.kind === "json") {
        throw new DiagramInputError(`Notes cannot target JSON node: ${target}`);
      }
      ensureUsecase(target);
      model.notes.push({ target, text: decodeUsecaseLabel(noteMatch[2] ?? "") });
      checkCount(model.notes.length, MAX_NODES, "notes");
      continue;
    }
    if (/^note\b/i.test(line)) {
      throw new DiagramInputError(`Unsupported note form: ${line.slice(0, 40)}`);
    }

    // Standalone metadata statement: `target@{ ... }`
    const metaTarget = /^([A-Za-z0-9_]+)@\{([\s\S]*)\}$/.exec(line);
    if (metaTarget) {
      const id = metaTarget[1] ?? "";
      const meta = parseSimpleMeta(metaTarget[2] ?? "");
      const boundary = boundaryById.get(id);
      if (boundary) {
        if (meta.type === "rect" || meta.type === "package") boundary.type = meta.type;
        else throw new DiagramInputError(`Unsupported boundary metadata on ${id}`);
        continue;
      }
      if (edgeIds.has(id)) continue;
      const elem = elemById.get(id);
      if (elem?.kind === "actor") {
        applyActorMeta(elem, metaTarget[2] ?? "");
        continue;
      }
      throw new DiagramInputError(`Unknown metadata target: ${id}`);
    }

    // Relationships:
    // 1. Include/Extend: `A [id@]..> : (include|extend) B`
    const incExt =
      /^([A-Za-z0-9_]+)\s+(?:([A-Za-z0-9_]+)@)?(\.{2,}>)\s*:\s*(include|extend)\s+([A-Za-z0-9_]+)$/i.exec(
        line,
      );
    if (incExt) {
      if ((incExt[3] ?? "") !== "..>") {
        throw new DiagramInputError("Include and extend operators do not accept extra dots");
      }
      const edgeId = incExt[2];
      const kind = (incExt[4] ?? "").toLowerCase() as "include" | "extend";
      const from = incExt[1] ?? "";
      const to = incExt[5] ?? "";
      if (edgeId) edgeIds.add(edgeId);
      ensureUsecase(from);
      ensureUsecase(to);
      const rel: UsecaseRelation = {
        from,
        to,
        kind,
        startMarker: "none",
        endMarker: "arrow",
        dashed: true,
        minlen: 1,
        label: `«${kind}»`,
      };
      if (edgeId !== undefined) rel.id = edgeId;
      model.relations.push(rel);
      checkCount(model.relations.length, MAX_EDGES, "relationships");
      continue;
    }

    // 2. Generalization: `A [id@]--|> B`
    const genMatch = /^([A-Za-z0-9_]+)\s+(?:([A-Za-z0-9_]+)@)?(-{2,}\|>)\s+([A-Za-z0-9_]+)$/.exec(
      line,
    );
    if (genMatch) {
      if ((genMatch[3] ?? "") !== "--|>") {
        throw new DiagramInputError("Generalization operator does not accept extra dashes");
      }
      const edgeId = genMatch[2];
      const from = genMatch[1] ?? "";
      const to = genMatch[4] ?? "";
      if (edgeId) edgeIds.add(edgeId);
      ensureUsecase(from);
      ensureUsecase(to);
      const rel: UsecaseRelation = {
        from,
        to,
        kind: "generalize",
        startMarker: "none",
        endMarker: "triangle",
        dashed: false,
        minlen: 1,
        label: "",
      };
      if (edgeId !== undefined) rel.id = edgeId;
      model.relations.push(rel);
      checkCount(model.relations.length, MAX_EDGES, "relationships");
      continue;
    }

    // 3. Labelled solid association: `A [id@]-- label (-+>|o|x|--) B`
    const lblAssoc = readLabelledAssociation(line);
    if (lblAssoc) {
      const { edgeId, from, rightOp, to } = lblAssoc;
      const label = decodeUsecaseLabel(lblAssoc.label);
      const dashes = rightOp.replace(/>$/, "").length;
      if (edgeId) edgeIds.add(edgeId);
      ensureUsecase(from);
      ensureUsecase(to);
      const rel: UsecaseRelation = {
        from,
        to,
        kind: "assoc",
        startMarker: "none",
        endMarker: rightOp.endsWith(">") ? "arrow" : "none",
        dashed: false,
        minlen: Math.max(1, dashes - 1),
        label,
      };
      if (edgeId !== undefined) rel.id = edgeId;
      model.relations.push(rel);
      checkCount(model.relations.length, MAX_EDGES, "relationships");
      continue;
    }

    // 4. Unlabelled solid association: `A [id@](-->|<--|--|--o|o--|--x|x--) B`
    const solidAssoc =
      /^([A-Za-z0-9_]+)\s+(?:([A-Za-z0-9_]+)@)?(<--+|-{2,}>|--o|o--|--x|x--|-{2,})\s+([A-Za-z0-9_]+)$/.exec(
        line,
      );
    if (solidAssoc) {
      const edgeId = solidAssoc[2];
      const op = solidAssoc[3] ?? "--";
      const from = solidAssoc[1] ?? "";
      const to = solidAssoc[4] ?? "";
      if (edgeId) edgeIds.add(edgeId);
      ensureUsecase(from);
      ensureUsecase(to);
      let startMarker: RelMarker = "none";
      let endMarker: RelMarker = "none";
      let dashes = 2;
      if (op.endsWith(">")) {
        endMarker = "arrow";
        dashes = op.length - 1;
      } else if (op.startsWith("<")) {
        startMarker = "arrow";
        dashes = op.length - 1;
      } else if (op === "--o") endMarker = "circle";
      else if (op === "o--") startMarker = "circle";
      else if (op === "--x") endMarker = "cross";
      else if (op === "x--") startMarker = "cross";
      else dashes = op.length;
      const rel: UsecaseRelation = {
        from,
        to,
        kind: "assoc",
        startMarker,
        endMarker,
        dashed: false,
        minlen: Math.max(1, dashes - 1),
        label: "",
      };
      if (edgeId !== undefined) rel.id = edgeId;
      model.relations.push(rel);
      checkCount(model.relations.length, MAX_EDGES, "relationships");
      continue;
    }

    const decl = parseElementDecl(line);
    if (decl) {
      registerDecl(decl, undefined);
      continue;
    }

    throw new DiagramInputError(`Unreadable usecase line: ${line.slice(0, 40)}`);
  }

  if (openBoundary) throw new DiagramInputError(`Unclosed systemBoundary: ${openBoundary.id}`);
  if (model.elements.length === 0 && model.boundaries.length === 0) {
    throw new DiagramInputError("A use case diagram needs actors, use cases, or boundaries");
  }

  // Post-parse semantic validation on relationship endpoints after upgrades/declarations settle.
  for (const rel of model.relations) {
    const a = elemById.get(rel.from);
    const b = elemById.get(rel.to);
    if (!a || !b) throw new DiagramInputError("Relationship references unknown element");
    if (a.kind === "json" || b.kind === "json") {
      const ok =
        rel.kind === "assoc" &&
        rel.startMarker !== "circle" &&
        rel.startMarker !== "cross" &&
        rel.endMarker !== "circle" &&
        rel.endMarker !== "cross";
      if (!ok) throw new DiagramInputError("JSON nodes allow only basic solid associations");
    }
    if (rel.kind === "include" || rel.kind === "extend") {
      if (a.kind !== "usecase" || b.kind !== "usecase") {
        throw new DiagramInputError(`${rel.kind} relationships require use case endpoints`);
      }
    }
    if (rel.kind === "generalize") {
      if (a.kind !== b.kind || a.kind === "json") {
        throw new DiagramInputError("Generalization requires two actors or two use cases");
      }
    }
  }

  return model;
}
