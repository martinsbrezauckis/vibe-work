/**
 * Long pastes in the message box: a paste over the threshold is kept aside and the box holds a short
 * marker in its place ("[Pasted text #1 +42 lines]"), shown as a chip above the box. On send every
 * marker is replaced by its text, so Vibe gets the message exactly as if the text had been pasted in full.
 * Depends on: nothing. Called by: the composer store (`takeContent`), Composer, PasteChips, tests.
 * Invariants: pure; a marker counts only while its exact text is in the draft and its paste is known, so
 * a marker the user edits, or one typed by hand for an unknown paste, is plain text.
 */

/** One long paste: its full text and the number it shows in its marker. */
export interface PasteItem {
  id: string;
  /** Shown in the marker and the chip ("#2"); counts up per message. */
  n: number;
  text: string;
}

/** A paste with more lines than this becomes a chip. */
const PASTE_MAX_LINES = 20;
/** A paste with more characters than this becomes a chip (a long single line, minified code). */
export const PASTE_MAX_CHARS = 2_000;

/** The number of lines of `text` (a final line break does not start a new line). */
export function lineCount(text: string): number {
  if (text === "") return 0;
  const breaks = text.split("\n").length - 1;
  return text.endsWith("\n") ? breaks : breaks + 1;
}

/** Line ends the way the text box keeps them, so a chip sends what a plain paste would have. */
export function normalizePaste(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

/** True when a paste of `text` becomes a chip. */
export function foldsIntoChip(text: string): boolean {
  return text.length > PASTE_MAX_CHARS || lineCount(text) > PASTE_MAX_LINES;
}

/** The marker that stands for `item` in the draft. */
export function pasteMarker(item: PasteItem): string {
  return `[Pasted text #${String(item.n)} +${String(lineCount(item.text))} lines]`;
}

/** The short chip label: "Pasted text, +42 lines". */
export function pasteLabel(item: PasteItem): string {
  return `Pasted text, +${String(lineCount(item.text))} lines`;
}

/** The pastes whose marker is in `draft`, in the order they appear. */
export function pastesIn(draft: string, pastes: readonly PasteItem[]): PasteItem[] {
  return pastes
    .map((item) => ({ item, at: draft.indexOf(pasteMarker(item)) }))
    .filter(({ at }) => at >= 0)
    .sort((a, b) => a.at - b.at)
    .map(({ item }) => item);
}

/** `draft` with every known marker replaced by its paste's full text. */
export function expandPastes(draft: string, pastes: readonly PasteItem[]): string {
  let text = draft;
  for (const item of pastes) text = text.split(pasteMarker(item)).join(item.text);
  return text;
}

/** The next marker number for a message holding `pastes`. */
export function nextPasteNumber(pastes: readonly PasteItem[]): number {
  return pastes.reduce((n, item) => Math.max(n, item.n), 0) + 1;
}

/**
 * The range of a whole marker that one Backspace (`back`) or Delete at the caret `at` removes, or
 * `null` when the caret is not right after (Backspace) or right before (Delete) a marker.
 */
export function markerAt(
  draft: string,
  at: number,
  pastes: readonly PasteItem[],
  back: boolean,
): { start: number; end: number; item: PasteItem } | null {
  for (const item of pastes) {
    const marker = pasteMarker(item);
    const start = back ? at - marker.length : at;
    if (start >= 0 && draft.startsWith(marker, start)) {
      return { start, end: start + marker.length, item };
    }
  }
  return null;
}

/** `draft` with the marker of `item` replaced by `by` ("" removes the chip, the text expands it). */
export function replaceMarker(draft: string, item: PasteItem, by: string): string {
  return draft.split(pasteMarker(item)).join(by);
}
