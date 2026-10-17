/** Plugin display labels shared by notices and Tools; pure text helpers, independent of plugin identity. */

/** A short, visibly escaped label for a manifest-controlled plugin name. */
export function displayPluginName(name: string): string {
  let label = "";
  let length = 0;
  let consumed = 0;
  for (const ch of name) {
    consumed += ch.length;
    // Keep this set aligned with vw_gate::is_hidden_char for native confirmations.
    const hidden = /[\p{Cc}\u061c\u200b-\u200f\u2028-\u202e\u2066-\u2069\ufeff]/u.test(ch);
    const token = hidden
      ? `\\u${(ch.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, "0")}`
      : ch;
    const width = hidden ? token.length : 1;
    if (length + width > (consumed < name.length ? 79 : 80)) return `${label}…`;
    label += token;
    length += width;
  }
  return label;
}
