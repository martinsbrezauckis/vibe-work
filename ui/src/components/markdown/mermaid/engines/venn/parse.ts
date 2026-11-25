/**
 * Venn diagram parser. Grammar follows Mermaid's venn page: optional `title`, `set` declarations
 * (bare or quoted identifiers, optional `["Label"]`, optional `:size`), `union` intersections
 * referencing previously declared sets, and indented `text` nodes attached to the most recent
 * `set` or `union`. CSS styling (`style`, `:::`) and directives are refused.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_NODES, StepBudget, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** A text label node placed inside a set or union region. */
export interface VennTextNode {
  id: string;
  label: string;
}

/** A single named set in a Venn diagram. */
export interface VennSet {
  id: string;
  label: string;
  size?: number | undefined;
  texts: VennTextNode[];
}

/** An intersection region of two or more previously declared sets. */
export interface VennUnion {
  sets: string[];
  label?: string | undefined;
  size?: number | undefined;
  texts: VennTextNode[];
}

/** A parsed Venn diagram. */
export interface VennModel {
  title: string;
  sets: VennSet[];
  unions: VennUnion[];
}

const REFUSED = /^(?:style|classDef|cssClass|click|link|callback)\b/i;
const NUMBER = /^\d{1,12}(?:\.\d{1,12})?$/;

function readQuoted(text: string, start: number): { value: string; next: number } {
  let out = "";
  let i = start + 1;
  while (i < text.length) {
    const ch = text[i] ?? "";
    if (ch === "\\" && i + 1 < text.length) {
      out += text[i + 1] ?? "";
      i += 2;
      continue;
    }
    if (ch === '"') return { value: out, next: i + 1 };
    out += ch;
    i += 1;
  }
  throw new DiagramInputError("Unterminated quote in venn diagram");
}

function readIdent(text: string, start: number, stopChars: string): { id: string; next: number } {
  let i = start;
  while (i < text.length && /\s/.test(text[i] ?? "")) i += 1;
  if (i >= text.length) throw new DiagramInputError("Expected identifier in venn diagram");
  if (text[i] === '"') {
    const q = readQuoted(text, i);
    if (!q.value) throw new DiagramInputError("Empty quoted identifier in venn diagram");
    return { id: q.value, next: q.next };
  }
  let id = "";
  while (i < text.length) {
    const ch = text[i] ?? "";
    if (/\s/.test(ch) || stopChars.includes(ch)) break;
    id += ch;
    i += 1;
  }
  if (!id) throw new DiagramInputError("Expected identifier in venn diagram");
  return { id, next: i };
}

function parseLabelAndSize(
  text: string,
  start: number,
): { label?: string | undefined; size?: number | undefined } {
  let i = start;
  let label: string | undefined;
  let size: number | undefined;

  while (i < text.length && /\s/.test(text[i] ?? "")) i += 1;
  if (text[i] === "[") {
    const innerStart = i + 1;
    if (text[innerStart] === '"') {
      const q = readQuoted(text, innerStart);
      label = q.value;
      let after = q.next;
      while (after < text.length && /\s/.test(text[after] ?? "")) after += 1;
      if (text[after] !== "]") throw new DiagramInputError("Unclosed `[` label in venn diagram");
      i = after + 1;
    } else {
      const close = text.indexOf("]", innerStart);
      if (close < 0) throw new DiagramInputError("Unclosed `[` label in venn diagram");
      label = text.slice(innerStart, close).trim();
      i = close + 1;
    }
  }

  while (i < text.length && /\s/.test(text[i] ?? "")) i += 1;
  if (text[i] === ":") {
    const numText = text.slice(i + 1).trim();
    if (!NUMBER.test(numText)) {
      throw new DiagramInputError(`Invalid venn size: ${numText.slice(0, 30)}`);
    }
    const parsed = Number(numText);
    if (!Number.isFinite(parsed) || !(parsed > 0)) {
      throw new DiagramInputError(`Venn size must be positive: ${numText}`);
    }
    size = parsed;
    i = text.length;
  }

  if (text.slice(i).trim().length > 0) {
    throw new DiagramInputError(
      `Unexpected trailing venn syntax: ${text.slice(i).trim().slice(0, 30)}`,
    );
  }
  return { label, size };
}

/**
 * Parse `venn-beta` source (text or prepared lines); throws DiagramInputError on syntax errors,
 * undeclared sets in `union`, duplicate sets, orphan `text`, or refused styling.
 */
export function parseVenn(input: string | PreparedSource): VennModel {
  const [, ...body] = linesOf(input, "venn");
  const model: VennModel = { title: "", sets: [], unions: [] };
  const declared = new Set<string>();
  const budget = new StepBudget();
  let lastTarget: VennSet | VennUnion | undefined;
  let nodeCount = 0;

  for (const raw of body) {
    budget.tick();
    const line = raw.trim();
    if (!line) continue;
    if (REFUSED.test(line) || line.includes(":::")) {
      throw new DiagramInputError(`Not drawn: ${line.slice(0, 40)}`);
    }

    const titleMatch = /^title\s+(.+)$/i.exec(line);
    if (titleMatch) {
      const val = (titleMatch[1] ?? "").trim();
      model.title =
        val.startsWith('"') && val.endsWith('"') && val.length >= 2 ? val.slice(1, -1) : val;
      continue;
    }

    const setMatch = /^set\s+(.+)$/i.exec(line);
    if (setMatch) {
      const rest = setMatch[1] ?? "";
      const { id, next } = readIdent(rest, 0, "[:");
      if (declared.has(id)) throw new DiagramInputError(`Duplicate set id "${id}"`);
      const { label, size } = parseLabelAndSize(rest, next);
      declared.add(id);
      const entry: VennSet = { id, label: label ?? id, size, texts: [] };
      model.sets.push(entry);
      lastTarget = entry;
      nodeCount += 1;
      checkCount(nodeCount, MAX_NODES, "nodes");
      continue;
    }

    const unionMatch = /^union\s+(.+)$/i.exec(line);
    if (unionMatch) {
      const rest = unionMatch[1] ?? "";
      const ids: string[] = [];
      let pos = 0;
      while (pos < rest.length) {
        budget.tick();
        const { id, next } = readIdent(rest, pos, ",[:");
        ids.push(id);
        pos = next;
        while (pos < rest.length && /\s/.test(rest[pos] ?? "")) pos += 1;
        if (rest[pos] === ",") {
          pos += 1;
          continue;
        }
        break;
      }
      if (ids.length < 2) throw new DiagramInputError("A venn union needs at least two sets");
      if (new Set(ids).size !== ids.length) {
        throw new DiagramInputError("Duplicate set id inside venn union");
      }
      for (const id of ids) {
        if (!declared.has(id)) {
          throw new DiagramInputError(`Unknown set "${id}" in venn union`);
        }
      }
      const { label, size } = parseLabelAndSize(rest, pos);
      const entry: VennUnion = { sets: ids, label, size, texts: [] };
      model.unions.push(entry);
      lastTarget = entry;
      nodeCount += 1;
      checkCount(nodeCount, MAX_NODES, "nodes");
      checkCount(model.unions.length, MAX_EDGES, "unions");
      continue;
    }

    const textMatch = /^text\s+(.+)$/i.exec(line);
    if (textMatch) {
      if (!lastTarget) {
        throw new DiagramInputError("A venn `text` line must follow a `set` or `union`");
      }
      const rest = textMatch[1] ?? "";
      const { id, next } = readIdent(rest, 0, "[:");
      const { label } = parseLabelAndSize(rest, next);
      lastTarget.texts.push({ id, label: label ?? id });
      nodeCount += 1;
      checkCount(nodeCount, MAX_NODES, "nodes");
      continue;
    }

    throw new DiagramInputError(`Unreadable venn line: ${line.slice(0, 40)}`);
  }

  if (model.sets.length === 0) {
    throw new DiagramInputError("A venn diagram needs at least one set");
  }
  return model;
}
