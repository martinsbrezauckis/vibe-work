/** Visible permission-card text; used by every card adapter without changing execution or answer bytes. */
import type { FileDiff } from "../transcript/model/diff";

/** Escape controls and Unicode default-ignorables so they cannot hide or reorder an approved target. */
export function visibleText(text: string): string {
  return text.replace(/[\p{Cc}\p{Cf}\p{Default_Ignorable_Code_Point}\p{Zl}\p{Zp}]/gu, (char) =>
    char === "\n"
      ? char
      : `\\u{${(char.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, "0")}}`,
  );
}

/** Escape a diff only for presentation; its original path and content still identify the approved change. */
export function visibleDiff(diff: FileDiff): FileDiff {
  return {
    ...diff,
    path: visibleText(diff.path),
    lines: diff.lines.map((line) => ({ ...line, text: visibleText(line.text) })),
  };
}
