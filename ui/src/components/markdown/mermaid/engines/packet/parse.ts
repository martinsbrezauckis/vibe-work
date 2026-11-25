/**
 * Packet diagram parser. Grammar follows Mermaid's packet diagram page: optional `title`, bit
 * positions `n: "label"`, bit ranges `start-end: "label"`, auto-increment `+count: "label"`, and
 * inline `%%` comments outside quotes. Styling, links and callbacks are refused.
 */
import { linesOf } from "../../detect";
import { MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

const MAX_BITS = 8192;
const REFUSED = /^(?:click|style|classDef|linkStyle|class|link|callback)\b|:::/i;

/** One bit-range field in a packet diagram. */
export interface PacketField {
  start: number;
  end: number;
  label: string;
}

/** A parsed packet diagram. */
export interface PacketModel {
  title: string;
  bitsPerRow: number;
  fields: PacketField[];
}

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

function parseQuoted(raw: string): string {
  const text = raw.trim();
  if (!text.startsWith('"') || text.length < 2) {
    throw new DiagramInputError("Packet field label must be quoted");
  }
  let out = "";
  let closed = false;
  for (let i = 1; i < text.length; i++) {
    const ch = text[i];
    if (ch === "\\") {
      const next = text[i + 1];
      if (next === undefined) throw new DiagramInputError("Unterminated packet field label");
      out += next;
      i++;
      continue;
    }
    if (ch === '"') {
      closed = true;
      if (text.slice(i + 1).trim() !== "") {
        throw new DiagramInputError("Unexpected text after packet field label");
      }
      break;
    }
    out += ch ?? "";
  }
  if (!closed) throw new DiagramInputError("Unterminated packet field label");
  return out;
}

function unquoteOptional(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.startsWith('"')) return parseQuoted(trimmed);
  return trimmed;
}

/** Parse `packet` or `packet-beta` source; throws DiagramInputError or DiagramLimitError on problems. */
export function parsePacket(input: string | PreparedSource): PacketModel {
  const [, ...body] = linesOf(input, "packet");
  const model: PacketModel = { title: "", bitsPerRow: 32, fields: [] };
  let cursor = 0;

  for (const raw of body) {
    const line = stripInlineComment(raw);
    if (line === "") continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    if (/^acc(?:Title|Descr)\b/i.test(line)) continue;
    const titleMatch = /^title(?:\s*:)?\s+(.+)$/i.exec(line);
    if (titleMatch) {
      model.title = unquoteOptional(titleMatch[1] ?? "");
      continue;
    }

    const plusMatch = /^\+(\d+)\s*:\s*(.+)$/.exec(line);
    const rangeMatch = /^(\d+)\s*-\s*(\d+)\s*:\s*(.+)$/.exec(line);
    const singleMatch = /^(\d+)\s*:\s*(.+)$/.exec(line);

    let start: number;
    let end: number;
    let labelRaw: string;

    if (plusMatch) {
      const count = Number(plusMatch[1]);
      if (!Number.isSafeInteger(count) || count < 1) {
        throw new DiagramInputError("Packet bit count must be at least 1");
      }
      start = cursor;
      end = cursor + count - 1;
      labelRaw = plusMatch[2] ?? "";
    } else if (rangeMatch) {
      start = Number(rangeMatch[1]);
      end = Number(rangeMatch[2]);
      if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || end < start) {
        throw new DiagramInputError(`Invalid packet bit range: ${line.slice(0, 40)}`);
      }
      labelRaw = rangeMatch[3] ?? "";
    } else if (singleMatch) {
      start = Number(singleMatch[1]);
      end = start;
      if (!Number.isSafeInteger(start)) {
        throw new DiagramInputError(`Invalid packet bit index: ${line.slice(0, 40)}`);
      }
      labelRaw = singleMatch[2] ?? "";
    } else {
      throw new DiagramInputError(`Unreadable packet line: ${line.slice(0, 40)}`);
    }

    checkCount(end + 1, MAX_BITS, "bits");
    if (model.fields.some((f) => start <= f.end && end >= f.start)) {
      throw new DiagramInputError(`Overlapping packet bit range: ${String(start)}-${String(end)}`);
    }
    const label = parseQuoted(labelRaw);
    model.fields.push({ start, end, label });
    checkCount(model.fields.length, MAX_NODES, "fields");
    cursor = Math.max(cursor, end + 1);
  }

  if (model.fields.length === 0) {
    throw new DiagramInputError("A packet diagram needs fields");
  }
  return model;
}
