/**
 * Voice chat words: the instruction a voice turn
 * carries in Vibe's context channel, the reply turned into plain speech, and the short spoken prompts for an
 * approval or a question. Marks are removed only at word
 * edges, so "snake_case" stays as it is.
 * Adapted from ShellX.
 * Depends on: nothing. Called by: state/voiceChat, the composer model (`message.ts`), tests.
 * Invariants: pure; model output is untrusted text, it is only ever turned into words to speak, never run;
 * every result fits the core's `speak` limit (2,000 characters).
 */

/** The instruction sent with each voice turn as a context entry (never inside the user's message). */
export const VOICE_INSTRUCTION =
  "This message was spoken in voice chat and your answer will be read aloud. Answer as speech: concise and " +
  "conversational, about 6 sentences at most, no tables or code blocks; if you used tools, say in one or two " +
  "sentences what you did and what you found.";

/** The longest text sent to text-to-speech (the core refuses more than 2,000 characters). */
export const MAX_SPOKEN_CHARS = 1_800;

/** What a code block becomes when spoken (the brief's words). */
export const CODE_SAID = "I put the code in the chat.";
/** What a table becomes when spoken. */
export const TABLE_SAID = "There is a table in the chat.";
/** What ends an answer cut for length. */
export const REST_SAID = "The rest is in the chat.";

/** Removes Markdown table blocks (two or more lines starting with `|`), one sentence in their place. */
function dropTables(text: string): string {
  return text.replace(/(?:^[ \t]*\|.*\|[ \t]*(?:\n|$)){2,}/gm, `${TABLE_SAID}\n`);
}

/**
 * Removes emphasis marks at word edges only (`**bold**`, `_it_`, `~~gone~~`): `snake_case`, `a*b` and `a > b` stay.
 * Shared with narration (`state/narration` `speakable`).
 */
export function stripEmphasis(text: string): string {
  return text
    .replace(/(^|[\s(])(\*{1,3}|_{1,3}|~~)(?=\S)/g, "$1")
    .replace(/(\S)(\*{1,3}|_{1,3}|~~)(?=$|[\s.,;:!?)])/g, "$1");
}

/** Cuts `text` to `max` characters at the last sentence end, adding `REST_SAID`. */
function cap(text: string, max: number): string {
  if (text.length <= max) return text;
  const room = max - REST_SAID.length - 1;
  const head = text.slice(0, room);
  const end = Math.max(head.lastIndexOf(". "), head.lastIndexOf("! "), head.lastIndexOf("? "));
  const kept = end > room / 3 ? head.slice(0, end + 1) : `${head.slice(0, head.lastIndexOf(" "))}…`;
  return `${kept} ${REST_SAID}`;
}

/**
 * The final answer of a turn as words to speak: fenced code → one "I put the code in the chat." per run of blocks,
 * tables → one sentence, links → their text, bare URLs → "the link in the chat", images and emoji dropped, Markdown
 * marks removed only at word or line edges (underscores inside words and comparison signs stay), list items and
 * paragraphs become sentences. At most `MAX_SPOKEN_CHARS`; an empty result means there is nothing to say.
 */
export function spokenReply(answer: string, max = MAX_SPOKEN_CHARS): string {
  let t = answer.replace(/\r\n?/g, "\n");
  // Fenced code (closed, or left open at the end) and indented-only lines are code.
  t = t.replace(/```[\s\S]*?(?:```|$)|~~~[\s\S]*?(?:~~~|$)/g, `\n${CODE_SAID}\n`);
  t = dropTables(t);
  t = t.replace(/!\[[^\]]*\]\([^)]*\)/g, " ");
  t = t.replace(/\[([^\]]+)\]\([^)]+\)/g, "$1");
  t = t.replace(/<?https?:\/\/[^\s>)]+>?/g, "the link in the chat");
  t = t.replace(/`([^`\n]+)`/g, "$1");
  // Line-start marks: headings, quotes, list bullets and numbers, horizontal rules.
  t = t.replace(/^[ \t]*(?:#{1,6}[ \t]+|>[ \t]?|[-*+][ \t]+|\d+[.)][ \t]+)/gm, "");
  t = t.replace(/^[ \t]*(?:[-*_][ \t]*){3,}$/gm, "");
  t = stripEmphasis(t);
  t = t.replace(/[ \t]*\p{Extended_Pictographic}\u{FE0F}?/gu, "");
  // Repeated "code" sentences (several blocks in a row) are said once.
  const escaped = CODE_SAID.replace(/[.]/g, "\\.");
  t = t.replace(new RegExp(`(?:${escaped}\\s*){2,}`, "g"), `${CODE_SAID} `);
  // Lines become sentences: a line without an end mark gets a period, so the voice pauses between items.
  const sentences = t
    .split(/\n+/)
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter((line) => line !== "")
    .map((line) => (/[.!?:;…]$/.test(line) ? line : `${line}.`));
  return cap(sentences.join(" "), max);
}

/** Short text the voice says when an approval opens (the card in the chat is still how the user answers). */
export function approvalPrompt(title: string): string {
  const what = title.replace(/\s+/g, " ").trim().slice(0, 120);
  return what === ""
    ? "Vibe asks for your approval. Answer on the card in the chat."
    : `Vibe asks for your approval: ${what}. Answer on the card in the chat.`;
}

/** Short text the voice says when the agent asks a question. */
export function questionPrompt(question: string): string {
  const what = question.replace(/\s+/g, " ").trim().slice(0, 240);
  return what === ""
    ? "Vibe has a question for you. Answer on the card in the chat."
    : `Vibe asks: ${what} Answer on the card in the chat.`;
}

/** What the voice says when a voice turn failed (the details stay in the chat). */
export const FAILED_SAID = "That turn failed; the details are in the chat.";
