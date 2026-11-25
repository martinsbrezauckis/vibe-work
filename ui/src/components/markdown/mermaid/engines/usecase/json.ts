/**
 * Strict source-order JSON object parser and Mermaid label/entity decoder for use case diagrams.
 * Preserves exact JSON property order (including integer-like keys such as "2" before "1"),
 * overwrites duplicate keys while keeping first position, and flattens nested values to table rows.
 */
import { MAX_MEMBERS, checkCount } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError } from "../../types";

/** A flattened key/value row inside a use case `json` table. */
export interface JsonTableRow {
  key: string;
  value: string;
}

type JsonVal =
  | { kind: "scalar"; text: string }
  | { kind: "array"; items: JsonVal[] }
  | { kind: "object"; entries: { key: string; val: JsonVal }[] };

const ENTITIES: Record<string, string> = {
  quot: '"',
  apos: "'",
  amp: "&",
  lt: "<",
  gt: ">",
  nbsp: " ",
};

/** Decode Mermaid `#name;` / `#num;` entity codes without altering backslashes. */
export function decodeEntities(raw: string): string {
  return raw.replace(/#([a-z]+|\d+);/gi, (match, code: string) => {
    const lower = code.toLowerCase();
    if (/^\d+$/.test(lower)) {
      const cp = Number.parseInt(lower, 10);
      return Number.isFinite(cp) && cp >= 32 && cp <= 0x10ffff ? String.fromCodePoint(cp) : match;
    }
    return ENTITIES[lower] ?? match;
  });
}

/** Strip basic Markdown formatting markers (`**`, `*`, `` ` ``) from Mermaid Markdown strings. */
export function stripMarkdownMarkers(text: string): string {
  return text
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/\*([^*]+)\*/g, "$1")
    .replace(/`([^`]+)`/g, "$1");
}

/** Parse a quoted plain string (`"..."`), Markdown string (``"`...`"``), or unquoted text. */
export function decodeUsecaseLabel(raw: string): string {
  const t = raw.trim();
  if (t.startsWith('"`') && t.endsWith('`"') && t.length >= 4) {
    return clampLabel(stripMarkdownMarkers(decodeEntities(t.slice(2, -2))));
  }
  if (t.startsWith('"') && t.endsWith('"') && t.length >= 2) {
    return clampLabel(decodeEntities(t.slice(1, -1)));
  }
  return clampLabel(decodeEntities(t));
}

/** Generate Mermaid's deterministic identifier from a quoted title by replacing non-word chars with `_`. */
export function autoIdFromTitle(decodedTitle: string): string {
  const id = decodedTitle.replace(/[^A-Za-z0-9_]+/g, "_");
  let start = 0;
  let end = id.length;
  while (start < end && id[start] === "_") start++;
  while (end > start && id[end - 1] === "_") end--;
  return id.slice(start, end) || "item";
}

/** Parse strict JSON while retaining exact source key insertion order even for numeric keys. */
export function parseStrictJsonTable(raw: string): JsonTableRow[] {
  let i = 0;
  const len = raw.length;
  const ws = (): void => {
    while (i < len && /\s/.test(raw[i] ?? "")) i++;
  };
  const fail = (msg: string): never => {
    throw new DiagramInputError(`Invalid JSON table: ${msg}`);
  };

  const readString = (): string => {
    if (raw[i] !== '"') fail("expected string");
    i++;
    let out = "";
    while (i < len) {
      const ch = raw[i++] ?? "";
      if (ch === '"') return out;
      if (ch === "\\") {
        const esc = raw[i++];
        if (esc === undefined) fail("unterminated escape");
        if (esc === '"' || esc === "\\" || esc === "/") out += esc;
        else if (esc === "b") out += "\b";
        else if (esc === "f") out += "\f";
        else if (esc === "n") out += "\n";
        else if (esc === "r") out += "\r";
        else if (esc === "t") out += "\t";
        else if (esc === "u") {
          const hex = raw.slice(i, i + 4);
          if (!/^[0-9a-fA-F]{4}$/.test(hex)) fail("invalid unicode escape");
          out += String.fromCodePoint(Number.parseInt(hex, 16));
          i += 4;
        } else fail(`bad escape \\${esc ?? ""}`);
      } else {
        if ((ch.codePointAt(0) ?? 0) < 0x20) fail("unescaped control character");
        out += ch;
      }
    }
    return fail("unterminated string");
  };

  const readValue = (depth: number): JsonVal => {
    if (depth > 16) fail("nesting too deep");
    ws();
    const ch = raw[i];
    if (ch === '"') return { kind: "scalar", text: readString() };
    if (ch === "{") return readObject(depth + 1);
    if (ch === "[") {
      i++;
      ws();
      const items: JsonVal[] = [];
      if (raw[i] === "]") {
        i++;
        return { kind: "array", items };
      }
      for (;;) {
        items.push(readValue(depth + 1));
        ws();
        if (raw[i] === ",") {
          i++;
          continue;
        }
        if (raw[i] === "]") {
          i++;
          return { kind: "array", items };
        }
        fail("expected `,` or `]` in array");
      }
    }
    if (raw.startsWith("true", i)) {
      i += 4;
      return { kind: "scalar", text: "true" };
    }
    if (raw.startsWith("false", i)) {
      i += 5;
      return { kind: "scalar", text: "false" };
    }
    if (raw.startsWith("null", i)) {
      i += 4;
      return { kind: "scalar", text: "null" };
    }
    const numMatch = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(raw.slice(i));
    if (numMatch) {
      i += numMatch[0].length;
      return { kind: "scalar", text: numMatch[0] };
    }
    return fail(`unexpected token near ${raw.slice(i, i + 20)}`);
  };

  const readObject = (depth: number): Extract<JsonVal, { kind: "object" }> => {
    if (raw[i] !== "{") fail("expected `{`");
    i++;
    ws();
    const entries: { key: string; val: JsonVal }[] = [];
    const seen = new Map<string, number>();
    if (raw[i] === "}") {
      i++;
      return { kind: "object", entries };
    }
    for (;;) {
      ws();
      const key = readString();
      ws();
      if (raw[i] !== ":") fail("expected `:` after key");
      i++;
      const val = readValue(depth);
      const prevIdx = seen.get(key);
      if (prevIdx !== undefined) {
        entries[prevIdx] = { key, val };
      } else {
        seen.set(key, entries.length);
        entries.push({ key, val });
      }
      ws();
      if (raw[i] === ",") {
        i++;
        continue;
      }
      if (raw[i] === "}") {
        i++;
        return { kind: "object", entries };
      }
      fail("expected `,` or `}` in object");
    }
  };

  ws();
  const root = readObject(0);
  ws();
  if (i < len) fail("trailing characters after JSON object");

  const rows: JsonTableRow[] = [];
  const flatten = (prefix: string, val: JsonVal): void => {
    if (val.kind === "scalar") {
      rows.push({ key: clampLabel(prefix), value: clampLabel(val.text) });
      checkCount(rows.length, MAX_MEMBERS, "JSON table rows");
      return;
    }
    if (val.kind === "object") {
      if (val.entries.length === 0 && prefix) {
        rows.push({ key: clampLabel(prefix), value: "{}" });
        checkCount(rows.length, MAX_MEMBERS, "JSON table rows");
        return;
      }
      for (const entry of val.entries) {
        const nextPrefix = prefix ? `${prefix}.${entry.key}` : entry.key;
        flatten(nextPrefix, entry.val);
      }
      return;
    }
    if (val.items.length === 0 && prefix) {
      rows.push({ key: clampLabel(prefix), value: "[]" });
      checkCount(rows.length, MAX_MEMBERS, "JSON table rows");
      return;
    }
    val.items.forEach((item, idx) => {
      flatten(`${prefix}[${String(idx)}]`, item);
    });
  };
  flatten("", root);
  return rows;
}
