/**
 * The edit preview the core attaches to an open file edit or write card (`detail.effect.vwPreview`, made from the file
 * on disk when the card opened, `crates/vw-core-api/src/edit_preview.rs`): the changed lines with their old and new
 * line numbers in the file. Pure. Depends on entries.ts and the diff types; called by the approval card.
 * Invariants: an absent or malformed preview gives `null` (the card then shows the requested change without file
 * line numbers); counts come from the core (they include lines the bounded preview left out); what the bounded
 * preview left out (`more` lines, `cutChars` characters, a line's own `cut`) is kept so the card can mark it.
 */
import type { DiffLine, FileDiff } from "./diff";
import { type Obj, arr, num, obj, str } from "./entries";

/** A preview diff and how many of its lines the core left out. */
export interface EditPreview {
  diff: FileDiff;
  /** Lines the core left out after the shown ones. */
  more: number;
  /** Characters the core cut from the ends of shown lines. */
  cutChars: number;
}

const KINDS: readonly DiffLine["kind"][] = ["ctx", "add", "del", "gap"];

function previewLine(value: Obj | null): DiffLine | null {
  const kind = KINDS.find((known) => known === value?.["kind"]);
  if (value === null || kind === undefined) return null;
  const cut = num(value["cut"]) ?? 0;
  return {
    kind,
    oldNo: num(value["oldNo"]),
    newNo: num(value["newNo"]),
    text: typeof value["text"] === "string" ? value["text"] : "",
    ...(cut > 0 ? { cut } : {}),
  };
}

/** The preview of the gated effect, or `null` when the core attached none. */
export function editPreview(effect: Obj): EditPreview | null {
  const preview = obj(effect["vwPreview"]);
  if (preview === null || !Array.isArray(preview["lines"])) return null;
  // An empty list is a change that leaves the file as it is.
  const lines = arr(preview["lines"]).map((line) => previewLine(obj(line)));
  if (lines.some((line) => line === null)) return null;
  const kept = lines.filter((line): line is DiffLine => line !== null);
  const count = (kind: DiffLine["kind"]) => kept.filter((line) => line.kind === kind).length;
  return {
    diff: {
      path: str(obj(effect["input"])?.["filePath"]) ?? "",
      lines: kept,
      added: num(preview["added"]) ?? count("add"),
      removed: num(preview["removed"]) ?? count("del"),
      newFile: preview["newFile"] === true,
    },
    more: Math.max(0, num(preview["more"]) ?? 0),
    cutChars: Math.max(0, num(preview["cutChars"]) ?? 0),
  };
}
