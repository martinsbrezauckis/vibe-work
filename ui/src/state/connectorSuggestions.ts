/** Reads a completed connector suggestion from Vibe's native tool-result envelopes; transcript rows call it. */
import type { JsonValue } from "../protocol/core";
import {
  effectView,
  obj,
  outputOf,
  outputText,
  str,
  type Obj,
} from "../features/transcript/model/entries";

/** A suggestion is an offer to review a native catalog entry, never permission to connect. */
export interface ConnectorSuggestion {
  alias: string;
  name: string;
  reason: string;
}

function read(value: JsonValue | undefined, depth = 0): ConnectorSuggestion | null {
  if (depth > 5) return null;
  if (typeof value === "string") {
    try {
      return value.length > 64000 ? null : read(JSON.parse(value) as JsonValue, depth + 1);
    } catch {
      return null;
    }
  }
  const object = obj(value);
  if (object === null) return null;
  const suggestion = obj(object["connectorSuggestion"]);
  const alias = str(suggestion?.["alias"]);
  const name = str(suggestion?.["name"]);
  const reason = str(suggestion?.["reason"]);
  if (alias && name && reason) return { alias, name, reason };
  for (const key of ["structuredContent", "structured_content", "output", "value"]) {
    const nested = read(object[key], depth + 1);
    if (nested) return nested;
  }
  const content = object["content"];
  if (Array.isArray(content))
    for (const block of content) {
      const nested = read(obj(block)?.["text"], depth + 1);
      if (nested) return nested;
    }
  return null;
}

/** Only the dedicated tool's completed result can contribute a suggestion. */
export function connectorSuggestionOf(entry: Obj): ConnectorSuggestion | null {
  const view = effectView(entry);
  if (
    view.status !== "completed" ||
    !["connector_suggest", "mcp_vw.connector_suggest"].includes(view.toolName)
  )
    return null;
  return read(outputOf(entry) ?? outputText(entry));
}
