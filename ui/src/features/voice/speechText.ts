/** Plain speech text and sentence chunks for narration and one-time reading; called by the narration store. */
import { stripEmphasis } from "./voiceChatText";

/** Narration cleanup; one-time readings name skipped code blocks and keep up to 10,000 characters. */
export function speakable(text: string, limit = 2_000, describeCode = false): string {
  const plain = text
    .replace(/```[^\n`]*\n?([\s\S]*?)```/g, (_block: string, code: string) =>
      describeCode ? ` (code block, ${String(code.trimEnd().split("\n").length)} lines) ` : " ",
    )
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/^[ \t]*(?:#{1,6}[ \t]+|>[ \t]?|[-*+][ \t]+)/gm, "")
    .replace(/^[ \t]*\|.*$/gm, " ");
  return Array.from(stripEmphasis(plain).replace(/\s+/g, " ").trim()).slice(0, limit).join("");
}

/** Speech calls stay under the provider's character cap, preferring whole sentences in every language. */
export function readingChunks(text: string, truncated = false): string[] {
  const raw = Array.from(text);
  const plain = speakable(raw.slice(0, 10_000).join(""), 10_000, true);
  const chunks: string[] = [];
  let pending = "";
  const size = (value: string): number => Array.from(value).length;
  for (const { segment } of new Intl.Segmenter(undefined, { granularity: "sentence" }).segment(
    plain,
  )) {
    let sentence = segment.trim();
    if (pending !== "" && size(`${pending} ${sentence}`) > 2_000) {
      chunks.push(pending);
      pending = "";
    }
    while (size(sentence) > 2_000) {
      const chars = Array.from(sentence);
      const head = chars.slice(0, 2_000).join("");
      const space = head.lastIndexOf(" ");
      const cut = space > 0 ? size(head.slice(0, space)) : 2_000;
      chunks.push(chars.slice(0, cut).join("").trim());
      sentence = chars.slice(cut).join("").trim();
    }
    pending = pending === "" ? sentence : `${pending} ${sentence}`;
  }
  if (pending !== "") chunks.push(pending);
  if (truncated || raw.length > 10_000) chunks.push("The rest is on screen.");
  return chunks;
}
