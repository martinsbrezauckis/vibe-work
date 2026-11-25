/**
 * Kanban diagram parser. Follows Mermaid's kanban page: `kanban`, indentation-defined columns
 * and tasks (`id[Title]`, `[Title]`, or bare `id`), optional `@{ key: value, ... }` task metadata
 * (`assigned`, `ticket`, `priority`, and extra key-value pairs). Refuses links/styles/callbacks.
 */
import { linesOf } from "../../detect";
import { MAX_MEMBERS, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** Priority levels documented by Mermaid's kanban syntax. */
export type KanbanPriority = "Very High" | "High" | "Low" | "Very Low";

/** One metadata entry outside the core assigned/ticket/priority fields. */
export interface KanbanExtraMeta {
  key: string;
  value: string;
}

/** One task card inside a kanban column. */
export interface KanbanTask {
  id: string;
  title: string;
  assigned?: string;
  ticket?: string;
  priority?: KanbanPriority;
  extras: KanbanExtraMeta[];
}

/** One workflow stage column in a kanban board. */
export interface KanbanColumn {
  id: string;
  title: string;
  tasks: KanbanTask[];
}

/** Parsed kanban board ready for layout and drawing. */
export interface KanbanModel {
  columns: KanbanColumn[];
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style)\b|:::/i;
const IGNORED = /^(?:accTitle|accDescr)\b/i;

const PRIORITY_MAP: Record<string, KanbanPriority> = {
  "very high": "Very High",
  high: "High",
  low: "Low",
  "very low": "Very Low",
};

function indentOf(raw: string): number {
  let width = 0;
  for (const ch of raw) {
    if (ch === " ") width++;
    else if (ch === "\t") width += 2;
    else break;
  }
  return width;
}

function unquoteMeta(raw: string): string {
  const s = raw.trim();
  if (s.length === 0) throw new DiagramInputError("Empty kanban metadata value");
  const q = s[0];
  if (q === "'" || q === '"') {
    if (s.length < 2 || s[s.length - 1] !== q) {
      throw new DiagramInputError(`Unterminated quote in kanban metadata: ${s.slice(0, 30)}`);
    }
    return s.slice(1, -1);
  }
  return s;
}

function splitMetadataEntries(inner: string): string[] {
  const out: string[] = [];
  let start = 0;
  let quote: string | undefined;
  for (let i = 0; i < inner.length; i++) {
    const ch = inner[i];
    if (quote) {
      if (ch === "\\" && i + 1 < inner.length) {
        i++;
        continue;
      }
      if (ch === quote) quote = undefined;
      continue;
    }
    if (ch === "'" || ch === '"') {
      quote = ch;
      continue;
    }
    if (ch === ",") {
      out.push(inner.slice(start, i).trim());
      start = i + 1;
    }
  }
  if (quote) throw new DiagramInputError("Unterminated quote in kanban metadata");
  const tail = inner.slice(start).trim();
  if (tail !== "" || out.length > 0) out.push(tail);
  return out;
}

function parseMetadata(
  rawMeta: string,
): Pick<KanbanTask, "assigned" | "ticket" | "priority" | "extras"> {
  if (!rawMeta.startsWith("@{") || !rawMeta.endsWith("}")) {
    throw new DiagramInputError("Unclosed kanban metadata block");
  }
  const inner = rawMeta.slice(2, -1).trim();
  const res: Pick<KanbanTask, "assigned" | "ticket" | "priority" | "extras"> = { extras: [] };
  if (inner === "") return res;

  const parts = splitMetadataEntries(inner);
  checkCount(parts.length, MAX_MEMBERS, "metadata entries");
  for (const part of parts) {
    if (part === "") throw new DiagramInputError("Empty entry in kanban metadata");
    const colon = part.indexOf(":");
    if (colon <= 0) {
      throw new DiagramInputError(`Malformed kanban metadata entry: ${part.slice(0, 30)}`);
    }
    const key = part.slice(0, colon).trim();
    const val = unquoteMeta(part.slice(colon + 1));
    if (!/^[a-zA-Z0-9_-]+$/.test(key)) {
      throw new DiagramInputError(`Invalid kanban metadata key: ${key.slice(0, 20)}`);
    }
    const lowerKey = key.toLowerCase();
    if (lowerKey === "assigned") {
      res.assigned = val;
    } else if (lowerKey === "ticket") {
      res.ticket = val;
    } else if (lowerKey === "priority") {
      const mapped = PRIORITY_MAP[val.trim().toLowerCase()];
      if (!mapped) {
        throw new DiagramInputError(`Unsupported kanban priority: ${val.slice(0, 20)}`);
      }
      res.priority = mapped;
    } else {
      res.extras.push({ key, value: val });
    }
  }
  return res;
}

function parseBracketTitle(
  text: string,
  openIndex: number,
): { title: string; afterCloseIndex: number } {
  let i = openIndex + 1;
  while (i < text.length && text[i] === " ") i++;
  if (text[i] === '"') {
    i++;
    let out = "";
    let closedQuote = false;
    while (i < text.length) {
      const ch = text[i];
      if (ch === "\\" && i + 1 < text.length) {
        out += text[i + 1] ?? "";
        i += 2;
        continue;
      }
      if (ch === '"') {
        closedQuote = true;
        i++;
        break;
      }
      out += ch ?? "";
      i++;
    }
    if (!closedQuote) throw new DiagramInputError("Unclosed quote in kanban title");
    while (i < text.length && text[i] === " ") i++;
    if (text[i] !== "]") throw new DiagramInputError("Expected closing bracket in kanban item");
    return { title: out.trim(), afterCloseIndex: i + 1 };
  }
  const close = text.indexOf("]", openIndex + 1);
  if (close < 0) throw new DiagramInputError("Unclosed bracket in kanban item");
  return { title: text.slice(openIndex + 1, close).trim(), afterCloseIndex: close + 1 };
}

function parseNode(line: string, fallbackId: string): KanbanTask {
  const openBracket = line.indexOf("[");
  const metaStart = line.indexOf("@{");
  let id: string;
  let title: string;
  let rest = "";

  if (openBracket >= 0 && (metaStart < 0 || openBracket < metaStart)) {
    const rawId = line.slice(0, openBracket).trim();
    if (/\s/.test(rawId)) {
      throw new DiagramInputError(`Kanban id cannot contain spaces: ${rawId.slice(0, 30)}`);
    }
    const parsed = parseBracketTitle(line, openBracket);
    title = parsed.title;
    if (!title) throw new DiagramInputError("Kanban item title cannot be empty");
    id = rawId || fallbackId;
    rest = line.slice(parsed.afterCloseIndex).trim();
  } else if (metaStart >= 0) {
    const rawId = line.slice(0, metaStart).trim();
    if (!rawId || /[\][{}]/.test(rawId)) {
      throw new DiagramInputError(`Unreadable kanban item: ${line.slice(0, 40)}`);
    }
    id = rawId;
    title = rawId;
    rest = line.slice(metaStart).trim();
  } else {
    if (/[\][{}@]/.test(line)) {
      throw new DiagramInputError(`Unreadable kanban item: ${line.slice(0, 40)}`);
    }
    id = line;
    title = line;
  }

  if (rest !== "" && !rest.startsWith("@{")) {
    throw new DiagramInputError(`Unexpected trailing content in kanban item: ${rest.slice(0, 30)}`);
  }
  const meta = rest ? parseMetadata(rest) : { extras: [] };
  return { id, title, ...meta };
}

/** Parse `kanban` source (text or prepared lines); throws DiagramInputError on syntax/indent errors. */
export function parseKanban(input: string | PreparedSource): KanbanModel {
  const [header = "", ...body] = linesOf(input, "kanban");
  if (!/^kanban\s*$/i.test(header.trim())) {
    throw new DiagramInputError("Unreadable kanban header");
  }

  const columns: KanbanColumn[] = [];
  let columnIndent: number | undefined;
  let currentTaskIndent: number | undefined;
  let totalTasks = 0;
  let inAccDescrBlock = false;

  for (const raw of body) {
    const line = raw.trim();
    if (inAccDescrBlock) {
      if (line.includes("}")) inAccDescrBlock = false;
      continue;
    }
    if (/^accDescr\s*\{/i.test(line)) {
      if (!line.includes("}")) inAccDescrBlock = true;
      continue;
    }
    if (IGNORED.test(line)) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    const indent = indentOf(raw);
    if (columnIndent === undefined || indent === columnIndent) {
      columnIndent = indent;
      currentTaskIndent = undefined;
      const col = parseNode(line, `col_${String(columns.length + 1)}`);
      columns.push({ id: col.id, title: col.title, tasks: [] });
      checkCount(columns.length, MAX_NODES, "columns");
      continue;
    }

    if (indent < columnIndent) {
      throw new DiagramInputError(`Inconsistent kanban indentation on line: ${line.slice(0, 30)}`);
    }

    const currentCol = columns[columns.length - 1];
    if (!currentCol) {
      throw new DiagramInputError("Kanban task must appear inside a column");
    }
    if (currentTaskIndent === undefined) {
      currentTaskIndent = indent;
    } else if (indent !== currentTaskIndent) {
      throw new DiagramInputError(
        `Kanban supports only two levels (column and task): ${line.slice(0, 30)}`,
      );
    }

    totalTasks++;
    checkCount(totalTasks, MAX_NODES, "tasks");
    const task = parseNode(line, `task_${String(totalTasks)}`);
    currentCol.tasks.push(task);
  }

  if (columns.length === 0) {
    throw new DiagramInputError("A kanban diagram needs at least one column");
  }
  return { columns };
}
