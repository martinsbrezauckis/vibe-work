/**
 * Incremental speech for a cumulative assistant answer.
 * Depends on: voiceChatText. Called by: voiceChat and its focused tests.
 * Holds unfinished Markdown until it can be sanitized as a complete piece.
 */
import { CODE_SAID, MAX_SPOKEN_CHARS, REST_SAID, TABLE_SAID, spokenReply } from "./voiceChatText";

const RESERVED = MAX_SPOKEN_CHARS - REST_SAID.length - 1;
const ABBREVIATION = /\b(?:Mr|Mrs|Ms|Dr|Prof|Sr|Jr|St|vs|etc|e\.g|i\.e)\.$/i;

/** Share one budget across all assistant messages in a voice turn. */
export interface VoiceReplyBudget {
  chars: number;
  truncated: boolean;
}

function sentenceCut(raw: string, from: number): number {
  const cut = from;
  let inline = false;
  let bracket = 0;
  let link = 0;
  let emphasis = "";
  let lineStart = from === 0 || raw[from - 1] === "\n";
  for (let i = from; i < raw.length; i += 1) {
    if (lineStart) {
      const rest = raw.slice(i);
      const fence = rest.match(/^[ \t]*(```|~~~)/);
      if (fence) {
        const marker = fence[1] ?? "```";
        const close = raw
          .slice(i + fence[0].length)
          .search(new RegExp(`^\\s*${marker}[ \\t]*(?:\\n|$)`, "m"));
        if (close < 0) return cut;
        const closeAt = i + fence[0].length + close;
        const lineEnd = raw.indexOf("\n", closeAt);
        if (lineEnd < 0) return cut;
        i = lineEnd;
        return i + 1;
      }
      // A table may still acquire more rows in the next chunk.
      if (/^[ \t]*\|/.test(rest)) {
        let end = i;
        while (end < raw.length && /^[ \t]*\|/.test(raw.slice(end))) {
          const next = raw.indexOf("\n", end);
          if (next < 0) return cut;
          end = next + 1;
        }
        if (end === raw.length) return cut;
        return end;
      }
      const lineMark = rest.match(/^[ \t]*(?:#{1,6}[ \t]+|>[ \t]?|[-*+][ \t]+|\d+[.)][ \t]+)/);
      if (lineMark) {
        i += lineMark[0].length - 1;
        lineStart = false;
        continue;
      }
    }
    const c = raw[i] ?? "";
    lineStart = c === "\n";
    if (c === "`") {
      inline = !inline;
      continue;
    }
    if (inline) continue;
    if (c === "[") bracket += 1;
    if (c === "]" && bracket) bracket -= 1;
    if (c === "(" && i > from && raw[i - 1] === "]") link += 1;
    if (c === ")" && link) link -= 1;
    if (bracket || link) continue;
    if (raw.slice(i).match(/^https?:\/\//)) {
      const end = raw.slice(i).search(/\s/);
      if (end < 0) return cut;
      i += end - 1;
      continue;
    }
    if (c === "*" || c === "_") {
      // A word-internal mark is ordinary text, not Markdown emphasis.
      if (!/\w/.test(raw[i - 1] ?? "") || !/\w/.test(raw[i + 1] ?? "")) {
        emphasis = emphasis ? "" : c;
      }
      while (raw[i + 1] === c) i += 1;
    }
    if (!".!?;".includes(c)) continue;
    if (c === "." && /\d/.test(raw[i - 1] ?? "") && /\d/.test(raw[i + 1] ?? "")) continue;
    if (c === "." && ABBREVIATION.test(raw.slice(from, i + 1))) continue;
    let end = i + 1;
    while (/[\]})"'’*_~]/.test(raw[end] ?? "")) end += 1;
    // The next chunk may turn `1.` into `1.5` or append Markdown closers.
    if (end === raw.length || !/\s/.test(raw[end] ?? "")) continue;
    if (emphasis && !raw.slice(i + 1, end).includes(emphasis)) continue;
    return end;
  }
  return cut;
}

function safeSpeech(raw: string): string {
  // Fenced code stays in the chat; completed inline spans keep their useful words.
  const withoutFences = raw.replace(/```[\s\S]*?(?:```|$)|~~~[\s\S]*?(?:~~~|$)/g, CODE_SAID);
  const withoutTables = withoutFences.replace(/(?:^[ \t]*\|[^\n]*(?:\n|$))+/gm, `${TABLE_SAID}\n`);
  return spokenReply(withoutTables, Number.MAX_SAFE_INTEGER);
}

/** Returns only new, speakable sentences. `raw` is the entire answer so far. */
export function createVoiceReplyStream(
  budget?: VoiceReplyBudget,
): (raw: string, done?: boolean) => string[] {
  let emittedRaw = "";
  const usage = budget ?? { chars: 0, truncated: false };
  let stopped = false;
  let revised = false;
  let lastWasCode = false;
  return (raw, done = false) => {
    if (stopped || usage.truncated) return [];
    if (!raw.startsWith(emittedRaw)) revised = true;
    if (revised) {
      // Already spoken words cannot be retracted. Wait for the replacement to settle, then label the correction.
      if (!done) return [];
      emittedRaw = "";
      raw = `Correction. ${raw}`;
    }
    const result: string[] = [];
    while (!stopped && emittedRaw.length < raw.length) {
      const safeCut = sentenceCut(raw, emittedRaw.length);
      const cut = safeCut > emittedRaw.length ? safeCut : done ? raw.length : 0;
      if (!cut) break;
      const finalPiece = done && cut === raw.length;
      let piece = raw.slice(emittedRaw.length, cut);
      if (finalPiece) {
        // Do not speak a partial link label or target if generation ended inside it.
        piece = piece.replace(/\[[^\]\n]*(?:\]\([^)]*)?$/, "");
        if ((piece.match(/`/g)?.length ?? 0) % 2 !== 0)
          piece = piece.slice(0, piece.lastIndexOf("`"));
      }
      let said = safeSpeech(piece);
      if (lastWasCode && said.startsWith(CODE_SAID)) {
        said = said.slice(CODE_SAID.length).trimStart();
      }
      const separator = usage.chars > 0 && said ? 1 : 0;
      const limit = finalPiece && !budget ? MAX_SPOKEN_CHARS : RESERVED;
      const available = limit - usage.chars - separator;
      if (said.length > available) {
        const room = RESERVED - usage.chars - separator;
        const prefix = said.slice(0, Math.max(0, room));
        const wordEnd = prefix.lastIndexOf(" ");
        const kept = wordEnd > prefix.length / 3 ? prefix.slice(0, wordEnd) : prefix;
        if (kept) {
          result.push(kept);
          usage.chars += kept.length + separator;
        }
        result.push(REST_SAID);
        usage.chars += REST_SAID.length + (usage.chars > 0 ? 1 : 0);
        usage.truncated = true;
        stopped = true;
      } else if (said) {
        result.push(said);
        usage.chars += said.length + separator;
        lastWasCode = said.endsWith(CODE_SAID);
      }
      emittedRaw = raw.slice(0, cut);
    }
    if (done) stopped = true;
    return result;
  };
}
