/**
 * Turns the queued messages the core took back from Vibe (`queueReturned`: a turn was stopped or failed while they
 * waited) into composer content: one draft text, the images and the context chips they carried.
 * Depends on: `message` (the composer item types). Called by: `state/composer` (the `queueReturned` listener).
 * Invariants: pure; the text of several messages is joined by a blank line, in queue order; only what the composer
 * itself put into a queued message comes back (`queueEntries`): text blocks, `image` blocks with a data URI, and the
 * inserted context (`vw://context/<n>`, `vw://doc/<id>`). Blocks the core adds on send (`vw://attachment/...`, the
 * attachment path notes) and the voice instruction (a `context` entry) are left out: a resend adds them again.
 */
import type { JsonValue } from "../../../protocol/core";
import type { ContextItem, ImageItem } from "./message";

/** What one hand-back puts into the composer. */
export interface QueuedBack {
  text: string;
  images: Omit<ImageItem, "id">[];
  context: Omit<ContextItem, "id">[];
}

type Json = Record<string, JsonValue | undefined>;

function object(value: JsonValue | undefined): Json | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

function list(value: JsonValue | undefined): JsonValue[] {
  return Array.isArray(value) ? value : [];
}

function str(value: JsonValue | undefined): string | null {
  return typeof value === "string" ? value : null;
}

/** Decoded size of base64 `data`. */
function decodedBytes(data: string): number {
  const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((data.length * 3) / 4) - padding);
}

/** The `vw.context` labels of a user entry's display annotations, in order. */
function contextLabels(entry: Json): { label: string; source: string }[] {
  const display = object(object(entry["annotations"])?.["vibe.userDisplayContent"]);
  return list(display?.["content"])
    .map(object)
    .filter((item): item is Json => item !== null && item["type"] === "vw.context")
    .map((item) => ({
      label: str(item["label"]) ?? "Context",
      source: str(item["source"]) ?? "quote",
    }));
}

/** The composer content of the queued `items` (Vibe's `TurnQueue.items`, oldest first). */
export function queuedBack(items: readonly JsonValue[]): QueuedBack {
  const texts: string[] = [];
  const images: Omit<ImageItem, "id">[] = [];
  const context: Omit<ContextItem, "id">[] = [];
  for (const item of items) {
    for (const entry of list(object(item)?.["entries"]).map(object)) {
      if (entry === null || entry["role"] !== "user") continue;
      const labels = contextLabels(entry);
      let chips = 0;
      const parts: string[] = [];
      for (const block of list(entry["content"]).map(object)) {
        if (block === null) continue;
        const text = str(block["text"]);
        const uri = str(block["uri"]) ?? "";
        if (block["type"] === "text" && text !== null) parts.push(text);
        else if (block["type"] === "image") {
          const found = /^data:([^;,]+);base64,(.*)$/s.exec(uri);
          if (found !== null) {
            const [, mimeType = "", data = ""] = found;
            images.push({
              name: str(block["altText"]) ?? "image",
              mimeType,
              data,
              bytes: decodedBytes(data),
            });
          }
        } else if (
          block["type"] === "embedded_resource" &&
          text !== null &&
          (uri.startsWith("vw://context/") || uri.startsWith("vw://doc/"))
        ) {
          const { label, source } = labels[chips] ?? { label: "Context", source: "quote" };
          chips += 1;
          context.push({
            label,
            source,
            text,
            ...(uri.startsWith("vw://doc/") ? { uri } : {}),
          });
        }
      }
      const joined = parts.join("\n").trim();
      if (joined !== "") texts.push(joined);
    }
  }
  return { text: texts.join("\n\n"), images, context };
}
