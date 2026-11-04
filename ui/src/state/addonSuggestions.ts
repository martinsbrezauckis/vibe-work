/** Reads a completed add-on suggestion (`addon_suggest`) from Vibe's native tool-result envelopes; transcript rows call it. */
import type { JsonValue } from "../protocol/core";
import {
  effectView,
  obj,
  outputOf,
  outputText,
  str,
  type Obj,
} from "../features/transcript/model/entries";

/** A suggestion shows the add-on's own control for the person; it never sets anything up by itself. */
export interface AddonSuggestion {
  addon: "chrome" | "edge";
  reason: string;
}

function read(value: JsonValue | undefined, depth = 0): AddonSuggestion | null {
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
  const suggestion = obj(object["addonSuggestion"]);
  const addon = str(suggestion?.["addon"]);
  const reason = str(suggestion?.["reason"]);
  if ((addon === "chrome" || addon === "edge") && reason) return { addon, reason };
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
export function addonSuggestionOf(entry: Obj): AddonSuggestion | null {
  const view = effectView(entry);
  if (
    view.status !== "completed" ||
    !["addon_suggest", "mcp_vw.addon_suggest"].includes(view.toolName)
  )
    return null;
  return read(outputOf(entry) ?? outputText(entry));
}
