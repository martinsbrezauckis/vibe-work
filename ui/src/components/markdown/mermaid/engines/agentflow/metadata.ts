/** Metadata entry splitting for the agentflow parser; preserves quote-parity delimiters. */

/** Split at every newline and at commas followed by an even number of quote characters. */
export function splitMetadata(raw: string): string[] {
  const parts: string[] = [];
  let end = raw.length;
  let evenQuotes = true;
  // Scan from the suffix: both quote kinds count, including escaped quotes.
  // Newlines always split entries without resetting the suffix's quote parity.
  for (let i = raw.length - 1; i >= 0; i--) {
    const ch = raw[i];
    if (ch === '"' || ch === "'") evenQuotes = !evenQuotes;
    else if (ch === "\n" || (ch === "," && evenQuotes)) {
      parts.push(raw.slice(i + 1, end));
      end = i;
    }
  }
  parts.push(raw.slice(0, end));
  return parts.reverse();
}
