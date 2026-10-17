/** Plugin display labels shared by notices and Tools; pure text helpers, independent of plugin identity. */

// Keep this set aligned with vw_gate::is_hidden_char for native confirmations.
const HIDDEN = /[\p{Cc}\u061c\u200b-\u200f\u2028-\u202e\u2066-\u2069\ufeff]/u;
// Characters that can open or close a quotation in some writing style. Keep this set aligned with
// vw_gate::is_quote_char, which writes the same label in native confirmations.
const QUOTE_LIKE =
  /["'`\u00ab\u00bb\u02ba\u02bc\u02ee\u05f4\u2018-\u201f\u2032-\u2037\u2039\u203a\u2057\u275b-\u275e\u2e42\u300c-\u300f\u301d-\u301f\uff02\uff07\uff62\uff63\u{1f676}-\u{1f678}]/u;

/** A short, visibly escaped label for a manifest-controlled plugin name. Hidden and quote-like characters
 * are written as `\uXXXX`, so a label placed between quotation marks can never end them early. */
export function displayPluginName(name: string): string {
  let label = "";
  let length = 0;
  let consumed = 0;
  for (const ch of name) {
    consumed += ch.length;
    const escaped = HIDDEN.test(ch) || QUOTE_LIKE.test(ch);
    const token = escaped
      ? `\\u${(ch.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, "0")}`
      : ch;
    const width = escaped ? token.length : 1;
    if (length + width > (consumed < name.length ? 79 : 80)) return `${label}…`;
    label += token;
    length += width;
  }
  return label;
}
