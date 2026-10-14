/**
 * An inline diff (the `.diff` layout: file header with +/- counts, numbered lines, added and
 * removed lines tinted). Built from Vibe's own effect data (`model/diff.ts`), no git. Long diffs show
 * the first `DIFF_LINES` lines with "Show all". Hunks are separated by a gap line.
 * Each diff is a focusable region with an accessible name. Accept and reject are handled by the Changes tab.
 * Adapted from ShellX.
 * Depends on: model/diff, the transcript store. Called by: step bodies and the approval card; the
 * Changes tab uses `DiffView` for edits in folders without git and `DiffLineRow` for hunks.
 */
import type { ReactElement } from "react";

import { useApp } from "../../shell/AppContext";
import { useTranscriptUi } from "../hooks";
import type { DiffLine, FileDiff } from "../model/diff";

/** Lines shown before "Show all". */
export const DIFF_LINES = 400;

function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** One numbered diff line (also used by the Changes tab's hunks: one diff row, not a fork). */
export function DiffLineRow({ line }: { line: DiffLine }): ReactElement {
  if (line.kind === "gap") {
    return (
      <div className="dl gap" aria-label="unchanged lines skipped">
        ⋯
      </div>
    );
  }
  const sign = line.kind === "add" ? "+" : line.kind === "del" ? "-" : "";
  const cls = line.kind === "add" ? "dl a" : line.kind === "del" ? "dl d" : "dl";
  return (
    <div className={cls} data-kind={line.kind}>
      <span className="n">{line.newNo ?? line.oldNo ?? ""}</span>
      <span className="s">{sign}</span>
      <span className="c">{line.text}</span>
    </div>
  );
}

/** Renders one file's diff. `id` keys its "show all" state. */
export function DiffView({ diff, id }: { diff: FileDiff; id: string }): ReactElement {
  const { transcript } = useApp();
  const key = `diff:${id}`;
  const full = useTranscriptUi((s) => s.open[key] === true);
  const lines = full ? diff.lines : diff.lines.slice(0, DIFF_LINES);
  const hidden = diff.lines.length - lines.length;
  const stat = diff.newFile
    ? `new file · +${String(diff.added)}`
    : `+${String(diff.added)} −${String(diff.removed)}`;
  return (
    <div
      className="diff"
      role="region"
      aria-label={`Changes to ${baseName(diff.path) || "the file"}`}
      tabIndex={0}
    >
      <div className="fh">
        <span className="fh-path" title={diff.path}>
          {diff.path || "(no path)"}
        </span>
        <span className="fh-stat">{stat}</span>
      </div>
      {lines.map((line, i) => (
        <DiffLineRow key={i} line={line} />
      ))}
      {hidden > 0 ? (
        <button
          type="button"
          className="show-all-btn"
          aria-label="Show full diff"
          title="Show full diff"
          onClick={() => {
            transcript.toggle(key, true);
          }}
        >
          Show all ({String(hidden)} more lines)
        </button>
      ) : null}
    </div>
  );
}
