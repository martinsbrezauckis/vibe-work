/**
 * Timeline diagram parser. Follows Mermaid's timeline page: `timeline [LR|TD]`, optional `title`,
 * `section <name>`, `{period} : {event} [: {event}...]` and `: {event}` continuation lines.
 * Refuses styling, callbacks, and click directives; ignores accTitle/accDescr accessibility lines.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** One time period and its ordered event descriptions. */
export interface TimelinePeriod {
  label: string;
  events: string[];
}

/** A named section (or the default implicit section) containing time periods. */
export interface TimelineSection {
  name: string;
  periods: TimelinePeriod[];
}

/** Parsed timeline diagram ready for layout and drawing. */
export interface TimelineModel {
  direction: "LR" | "TD";
  title: string;
  hasExplicitSections: boolean;
  sections: TimelineSection[];
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style)\b|:::/i;
const IGNORED = /^(?:accTitle|accDescr)\b/i;

/**
 * Split on timeline `:` delimiters without breaking `://`, `javascript:...` or colons inside
 * parentheses, while preserving empty trailing/consecutive delimiters as syntax errors.
 */
function splitTimelineSegments(line: string): string[] {
  const segments: string[] = [];
  let start = 0;
  let parenDepth = 0;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === "(") {
      parenDepth++;
      continue;
    }
    if (ch === ")" && parenDepth > 0) {
      parenDepth--;
      continue;
    }
    if (ch !== ":" || parenDepth > 0) continue;
    const prev = line[i - 1] ?? "";
    const next = line[i + 1] ?? "";
    if (line.slice(i, i + 3) === "://") continue;
    const isDelimiter = i === 0 || i === line.length - 1 || /\s/.test(prev) || /\s/.test(next);
    if (!isDelimiter) continue;
    segments.push(line.slice(start, i).trim());
    start = i + 1;
  }
  segments.push(line.slice(start).trim());
  return segments;
}

/** Parse `timeline` source (text or prepared lines); throws DiagramInputError on syntax errors. */
export function parseTimeline(input: string | PreparedSource): TimelineModel {
  const [header = "", ...body] = linesOf(input, "timeline");
  const head = /^timeline(?:\s+(LR|TD))?\s*$/i.exec(header.trim());
  if (!head) {
    throw new DiagramInputError(`Unreadable timeline header: ${header.trim().slice(0, 40)}`);
  }
  const direction: "LR" | "TD" = (head[1] ?? "LR").toUpperCase() === "TD" ? "TD" : "LR";
  const model: TimelineModel = {
    direction,
    title: "",
    hasExplicitSections: false,
    sections: [{ name: "", periods: [] }],
  };
  let totalPeriods = 0;
  let totalEvents = 0;
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
    if (/^title(?:\s|$)/i.test(line)) {
      const title = line.slice(5).trim();
      if (!title) throw new DiagramInputError("Timeline title cannot be empty");
      model.title = title;
      continue;
    }
    if (/^section(?:\s|$)/i.test(line)) {
      const name = line.slice(7).trim();
      if (!name) throw new DiagramInputError("Timeline section needs a name");
      model.hasExplicitSections = true;
      model.sections.push({ name, periods: [] });
      checkCount(model.sections.length, MAX_NODES, "sections");
      continue;
    }
    const parts = splitTimelineSegments(line);
    if (parts.length < 2) {
      throw new DiagramInputError(`Unreadable timeline line: ${line.slice(0, 40)}`);
    }
    const currentSection = model.sections[model.sections.length - 1];
    if (!currentSection) throw new DiagramInputError("Missing timeline section");
    if (line.startsWith(":")) {
      const lastPeriod = currentSection.periods[currentSection.periods.length - 1];
      if (!lastPeriod) {
        throw new DiagramInputError("Timeline event continuation needs a preceding time period");
      }
      const added = parts.slice(1);
      if (added.some((ev) => ev === "")) {
        throw new DiagramInputError(`Empty timeline event: ${line.slice(0, 40)}`);
      }
      lastPeriod.events.push(...added);
      totalEvents += added.length;
      checkCount(totalEvents, MAX_EDGES, "events");
      continue;
    }
    const label = parts[0] ?? "";
    const events = parts.slice(1);
    if (!label || events.length === 0 || events.some((ev) => ev === "")) {
      throw new DiagramInputError(`Timeline period needs a label and event: ${line.slice(0, 40)}`);
    }
    currentSection.periods.push({ label, events });
    totalPeriods++;
    totalEvents += events.length;
    checkCount(totalPeriods, MAX_NODES, "periods");
    checkCount(totalEvents, MAX_EDGES, "events");
  }

  if (totalPeriods === 0) {
    throw new DiagramInputError("A timeline diagram needs at least one time period");
  }
  return model;
}
