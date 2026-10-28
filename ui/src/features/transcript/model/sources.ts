/**
 * Web search sources. Two shapes, both from Vibe 2.26.0:
 * - Vibe's built-in `web_search` tool (`core/tools/builtins/web_search.py`), projected as an effect with
 *   `detail.kind: "web_search"`, `detail.input.query` and, once completed, `state.output = {query, answer,
 *   sources: [{title, url}]}` (`app_server/_unified_tool_projection.py:791-822`);
 * - Mistral's hosted search connector, which uses this shape: `detail.kind: "tool"`, `detail.toolName:
 *   "connector_web_search.web_search"` (or `news_search`), `detail.input.query`, and `state.output =
 *   {type, content: [{type: "text", text: <JSON of the refs>}], structured_content: {<ref>: {title, url,
 *   rank, description, snippets, metadata: {favicon, canonical_link?,...},...}}, _meta}`; the page is
 *   `metadata.canonical_link` when given (news results carry an internal id in `url`), else `url`.
 * This module reads the sources for the search step and gathers them for the "Sources (n)" chip of the
 * answer that follows the searches.
 * Depends on: entries.ts, links.ts. Called by: build.ts (which answer gets which searches), the step
 * body, the answer row, the sources list, the `ui/transcript` route.
 * Invariants: pure; never fetches anything (no favicons, no previews: the app does not load the pages);
 * sources are deduplicated by URL in first-seen order; a source whose URL cannot be opened keeps its
 * entry (the link dialog then says why) but has no host.
 */
import { checkLink } from "./links";
import { type Obj, arr, kindOf, num, obj, outputOf, str } from "./entries";

/** One source as the transcript shows it. */
export interface Source {
  /** The page title Mistral gave (untrusted text). */
  title: string;
  /** The URL exactly as Vibe reported it. */
  url: string;
  /** The host to show (punycode for internationalised names), or `null` when it cannot be opened. */
  host: string | null;
}

/** Mistral's hosted search connector tools whose output lists pages (see the module header). */
const CONNECTOR_SEARCH_TOOLS: ReadonlySet<string> = new Set([
  "connector_web_search.web_search",
  "connector_web_search.news_search",
]);

/** True for a web search effect (built-in tool or the hosted search connector). */
export function isWebSearch(entry: Obj): boolean {
  if (entry["type"] !== "effect") return false;
  const kind = kindOf(entry);
  const tool = str(obj(entry["detail"])?.["toolName"]) ?? "";
  return kind === "web_search" || (kind === "tool" && CONNECTOR_SEARCH_TOOLS.has(tool));
}

/** The connector's refs: `structured_content`, else the JSON text of the first text block. */
function connectorRefs(entry: Obj): Obj | null {
  const out = outputOf(entry);
  const structured = obj(out?.["structured_content"]);
  if (structured !== null) return structured;
  for (const raw of arr(out?.["content"])) {
    const block = obj(raw);
    const text = block?.["type"] === "text" ? str(block["text"]) : null;
    if (text === null) continue;
    try {
      const parsed: unknown = JSON.parse(text);
      if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed))
        return parsed as Obj;
    } catch {
      return null;
    }
  }
  return null;
}

/** The `{title, url}` records of one search effect, in Mistral's order (connector: by `rank`). */
function rawSources(entry: Obj): Obj[] {
  if (kindOf(entry) === "web_search") {
    return arr(outputOf(entry)?.["sources"]).flatMap((raw) => {
      const o = obj(raw);
      return o === null ? [] : [o];
    });
  }
  const refs = connectorRefs(entry);
  if (refs === null) return [];
  // `news_search` gives an internal id in `url` and the page in `metadata.canonical_link` (live run
  // `web_search` provides the same fields.
  const meta = obj(obj(outputOf(entry)?.["_meta"])?.["references_metadata"]);
  const list = Object.entries(refs).flatMap(([ref, raw]) => {
    const o = obj(raw);
    if (o === null) return [];
    const canonical =
      str(obj(o["metadata"])?.["canonical_link"]) ?? str(obj(meta?.[ref])?.["canonical_link"]);
    return [canonical === null || canonical === "" ? o : { ...o, url: canonical }];
  });

  const rank = (o: Obj) => num(o["rank"]) ?? Number.MAX_SAFE_INTEGER;
  return list.sort((a, b) => rank(a) - rank(b));
}

/** The query of a web search effect ("" when absent). */
export function searchQuery(entry: Obj): string {
  return (
    str(obj(obj(entry["detail"])?.["input"])?.["query"]) ?? str(outputOf(entry)?.["query"]) ?? ""
  );
}

/** The answer text Mistral's search agent wrote ("" when absent). */
export function searchAnswer(entry: Obj): string {
  return str(outputOf(entry)?.["answer"]) ?? "";
}

/** The sources of one web search effect, deduplicated by URL. */
export function searchSources(entry: Obj): Source[] {
  return gatherSources([entry]);
}

/** The sources of several web search effects, deduplicated by URL in order. */
export function gatherSources(entries: readonly Obj[]): Source[] {
  const seen = new Set<string>();
  const out: Source[] = [];
  for (const entry of entries) {
    for (const source of rawSources(entry)) {
      const url = str(source["url"]);
      if (url === null || url === "" || seen.has(url)) continue;
      seen.add(url);
      const check = checkLink(url);
      out.push({
        title: str(source["title"])?.trim() || url,
        url,
        host: check.ok ? check.host : null,
      });
    }
  }
  return out;
}
