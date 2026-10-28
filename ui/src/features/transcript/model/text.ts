/**
 * Long-text handling for the transcript: tool outputs and messages are cut before they reach the DOM
 * ("a 10 000-line tool output and a 2 MB message don't freeze the window"; the
 * live `state.output` carries no `truncated` flag, so the cut is ours). The full text stays in the
 * store and is rendered only when the user asks ("Show all").
 * Depends on: nothing. Called by: the step bodies, the message rows, the diff view.
 * Invariants: cuts never split a UTF-16 surrogate pair; the result says how much was left out.
 */

/** A cut text and what it left out. */
export interface Cut {
  text: string;
  /** True when anything was left out. */
  cut: boolean;
  /** Lines in the whole text. */
  totalLines: number;
  /** Lines left out (0 when only characters were trimmed). */
  hiddenLines: number;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

/** `text.slice(0, end)` moved back so it does not end inside a surrogate pair. */
export function safeHead(text: string, end: number): string {
  if (end >= text.length) return text;
  const cut = end > 0 && isHighSurrogate(text.charCodeAt(end - 1)) ? end - 1 : end;
  return text.slice(0, cut);
}

/** `text.slice(start)` moved forward so it does not start inside a surrogate pair. */
function safeTail(text: string, start: number): string {
  if (start <= 0) return text;
  const from = start < text.length && isLowSurrogate(text.charCodeAt(start)) ? start + 1 : start;
  return text.slice(from);
}

/** Counts lines without splitting a large text into an array. */
export function lineCount(text: string): number {
  if (text === "") return 0;
  let lines = 1;
  for (let i = text.indexOf("\n"); i !== -1; i = text.indexOf("\n", i + 1)) lines += 1;
  return text.endsWith("\n") ? lines - 1 : lines;
}

/** The first `maxLines` lines and at most `maxChars` characters of `text`. */
export function headText(text: string, maxLines: number, maxChars: number): Cut {
  const totalLines = lineCount(text);
  let end = 0;
  for (let n = 0; n < maxLines; n += 1) {
    const next = text.indexOf("\n", end);
    if (next === -1) {
      end = text.length;
      break;
    }
    end = next + 1;
  }
  const head = safeHead(text, Math.min(end === 0 ? text.length : end, maxChars));
  const shownLines = lineCount(head);
  return {
    text: head,
    cut: head.length < text.length,
    totalLines,
    hiddenLines: Math.max(0, totalLines - shownLines),
  };
}

/** The last `maxLines` lines and at most `maxChars` characters of `text` (logs: the end matters). */
export function tailText(text: string, maxLines: number, maxChars: number): Cut {
  const totalLines = lineCount(text);
  const body = text.endsWith("\n") ? text.slice(0, -1) : text;
  let start = body.length;
  let pos = body.length;
  for (let n = 0; n < maxLines; n += 1) {
    const prev = pos === 0 ? -1 : body.lastIndexOf("\n", pos - 1);
    if (prev === -1) {
      start = 0;
      break;
    }
    start = prev + 1;
    pos = prev;
  }
  const tail = safeTail(body, Math.max(start, body.length - maxChars));
  const shownLines = lineCount(tail);
  return {
    text: tail,
    cut: tail.length < body.length,
    totalLines,
    hiddenLines: Math.max(0, totalLines - shownLines),
  };
}
