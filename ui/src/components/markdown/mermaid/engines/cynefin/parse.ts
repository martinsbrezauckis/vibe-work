/**
 * Cynefin framework diagram parser. Grammar follows Mermaid's cynefin page: optional title, the
 * five fixed domain blocks (`complex`, `complicated`, `clear`, `chaotic`, `confusion`) holding
 * quoted items, and transitions `from --> to : label` (self-loops are silently ignored).
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_MEMBERS, MAX_NODES, checkCount } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError, type PreparedSource } from "../../types";

/** The five fixed Cynefin domains. */
export type CynefinDomain = "complex" | "complicated" | "clear" | "chaotic" | "confusion";

/** A transition arrow between two distinct Cynefin domains. */
export interface CynefinTransition {
  from: CynefinDomain;
  to: CynefinDomain;
  label: string;
}

/** A parsed Cynefin framework diagram. */
export interface CynefinModel {
  title: string;
  domains: Record<CynefinDomain, string[]>;
  transitions: CynefinTransition[];
}

const DOMAINS = new Set<CynefinDomain>(["complex", "complicated", "clear", "chaotic", "confusion"]);
const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style|linkStyle)\b|:::/i;
const TRANSITION = /^([a-z]+)\s*-->\s*([a-z]+)(?:\s*:\s*(.*))?$/i;

function unquote(raw: string): string {
  const text = raw.trim();
  if (!text.startsWith('"')) {
    throw new DiagramInputError(`Expected quoted item or label: ${text.slice(0, 40)}`);
  }
  if (text.length < 2 || !text.endsWith('"')) {
    throw new DiagramInputError("Unterminated quoted string");
  }
  return text.slice(1, -1).replace(/\\"/g, '"');
}

function parseOptionalLabel(raw: string | undefined): string {
  if (raw === undefined) return "";
  const trimmed = raw.trim();
  if (trimmed === "") return "";
  if (trimmed.startsWith('"')) return clampLabel(unquote(trimmed));
  return clampLabel(trimmed);
}

/** Parse `cynefin-beta` source (text or prepared lines); throws DiagramInputError on invalid or empty input. */
export function parseCynefin(input: string | PreparedSource): CynefinModel {
  const [, ...body] = linesOf(input, "cynefin");
  const model: CynefinModel = {
    title: "",
    domains: {
      complex: [],
      complicated: [],
      clear: [],
      chaotic: [],
      confusion: [],
    },
    transitions: [],
  };
  const declared = new Set<CynefinDomain>();
  let current: CynefinDomain | undefined;
  let totalItems = 0;
  let accDescrOpen = false;

  for (const raw of body) {
    const line = raw.trim();
    if (accDescrOpen) {
      if (line === "}") accDescrOpen = false;
      continue;
    }
    if (/^accDescr\s*\{$/i.test(line)) {
      accDescrOpen = true;
      continue;
    }
    if (/^(?:accTitle|accDescr)\b/i.test(line)) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }
    const titleMatch = /^title\s*:?\s*(.+)$/i.exec(line);
    if (titleMatch) {
      model.title = clampLabel((titleMatch[1] ?? "").trim());
      continue;
    }
    const domainCandidate = line.toLowerCase() as CynefinDomain;
    if (DOMAINS.has(domainCandidate)) {
      current = domainCandidate;
      declared.add(domainCandidate);
      continue;
    }
    const transMatch = TRANSITION.exec(line);
    if (transMatch) {
      current = undefined;
      const from = (transMatch[1] ?? "").toLowerCase() as CynefinDomain;
      const to = (transMatch[2] ?? "").toLowerCase() as CynefinDomain;
      if (!DOMAINS.has(from) || !DOMAINS.has(to)) {
        throw new DiagramInputError(`Unknown Cynefin domain in transition: ${line.slice(0, 40)}`);
      }
      const label = parseOptionalLabel(transMatch[3]);
      if (from === to) continue;
      model.transitions.push({ from, to, label });
      checkCount(model.transitions.length, MAX_EDGES, "transitions");
      declared.add(from);
      declared.add(to);
      continue;
    }
    if (line.startsWith('"')) {
      if (!current) {
        throw new DiagramInputError("Item must appear inside a domain block");
      }
      const item = clampLabel(unquote(line));
      model.domains[current].push(item);
      checkCount(model.domains[current].length, MAX_MEMBERS, `items in ${current}`);
      totalItems++;
      checkCount(totalItems, MAX_NODES, "items");
      continue;
    }
    throw new DiagramInputError(`Unreadable cynefin line: ${line.slice(0, 40)}`);
  }

  if (accDescrOpen) throw new DiagramInputError("Unclosed accDescr block");
  if (declared.size === 0 && model.transitions.length === 0) {
    throw new DiagramInputError("A cynefin diagram needs domains or transitions");
  }
  return model;
}
