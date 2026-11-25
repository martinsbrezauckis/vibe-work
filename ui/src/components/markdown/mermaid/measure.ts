/**
 * DOM-free text measuring. No canvas exists where diagrams are laid out, so widths come from a
 * glyph-class model of Inter (the UI font) and Space Mono, kept slightly generous so labels fit.
 */
import { MAX_LABEL_CHARS } from "./limits";

const NARROW = new Set("iIl1|.,:;!'`jt()[]{}/\\ -");
const MEDIUM_NARROW = new Set('frJ*"');
const WIDE = new Set("mwMW@%&");

/** Width of one code point as a fraction of the font size. */
function glyphEm(character: string, mono: boolean): number {
  if (mono) return 0.62;
  const code = character.codePointAt(0) ?? 0;
  if (code >= 0x1100) return 1.05;
  if (character === " ") return 0.3;
  if (NARROW.has(character)) return 0.33;
  if (MEDIUM_NARROW.has(character)) return 0.42;
  if (WIDE.has(character)) return 0.88;
  if (character >= "A" && character <= "Z") return 0.68;
  if (character >= "0" && character <= "9") return 0.6;
  return 0.57;
}

/** Font size and weight class used to estimate a width. */
export interface TextStyle {
  size: number;
  bold?: boolean;
  mono?: boolean;
}

/** Estimated width of a single line. */
export function textWidth(text: string, style: TextStyle): number {
  let em = 0;
  for (const character of text) em += glyphEm(character, style.mono === true);
  return em * style.size * (style.bold ? 1.06 : 1);
}

/** Cut text to the label cap with a visible ellipsis. */
export function clampLabel(text: string, max = MAX_LABEL_CHARS): string {
  const chars = Array.from(text);
  return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text;
}

/** Cut one line to a pixel width with an ellipsis. */
export function fitWidth(text: string, width: number, style: TextStyle): string {
  if (textWidth(text, style) <= width) return text;
  const chars = Array.from(text);
  while (chars.length > 1 && textWidth(`${chars.join("")}…`, style) > width) chars.pop();
  return `${chars.join("")}…`;
}

/**
 * Wrap on spaces to a width, honouring explicit `<br>` / `\n` breaks. A single word longer than the
 * width stays whole (the box grows); `maxLines` cuts the rest with an ellipsis.
 */
export function wrapText(text: string, maxWidth: number, style: TextStyle, maxLines = 8): string[] {
  const out: string[] = [];
  for (const paragraph of clampLabel(text).split(/<br\s*\/?>|\\n|\n/i)) {
    let line = "";
    for (const word of paragraph.split(/\s+/).filter(Boolean)) {
      const next = line ? `${line} ${word}` : word;
      if (line && textWidth(next, style) > maxWidth) {
        out.push(line);
        line = word;
      } else line = next;
    }
    out.push(line);
  }
  if (out.length > maxLines) {
    const kept = out.slice(0, maxLines);
    kept[maxLines - 1] = `${kept[maxLines - 1] ?? ""}…`;
    return kept;
  }
  return out;
}

/** Width and height of lines drawn at `style.size` with `lineHeight` (a multiple of the size). */
export function measureLines(
  lines: readonly string[],
  style: TextStyle,
  lineHeight = 1.3,
): { width: number; height: number } {
  let width = 0;
  for (const line of lines) width = Math.max(width, textWidth(line, style));
  return { width, height: lines.length * style.size * lineHeight };
}
