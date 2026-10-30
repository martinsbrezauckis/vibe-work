/**
 * An inline diff (the `.diff` layout: file header with +/- counts, numbered lines, added and
 * removed lines tinted). Built from Vibe's own effect data (`model/diff.ts`), no git. Long diffs show
 * the first `DIFF_LINES` lines with "Show all". Hunks are separated by a gap line.
 * Each diff is a focusable region with an accessible name. Accept and reject are handled by the Changes tab.
 * Adapted from ShellX.
 * Depends on: model/diff, the transcript store. Called by: step bodies and the approval card; the
 * Changes tab uses `DiffView` for edits in folders without git and `DiffLineRow` for hunks.
 */
import { Button } from "../../../components/Controls";
import type { ReactElement, ReactNode } from "react";

import { useApp } from "../../shell/AppContext";
import { useTranscriptUi } from "../hooks";
import type { DiffLine, FileDiff } from "../model/diff";

/** Lines shown before "Show all". */
export const DIFF_LINES = 400;

function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/**
 * One numbered diff line (also used by the Changes tab's hunks: one diff row, not a fork). With `both` it shows the
 * old and the new line number in two columns (a unified diff gutter), else one number (the new line, else the old).
 */
export function DiffLineRow({
  line,
  both = false,
}: {
  line: DiffLine;
  both?: boolean;
}): ReactElement {
  if (line.kind === "gap") {
    return (
      <div className={both ? "dl gap two" : "dl gap"} aria-label="unchanged lines skipped">
        ⋯
      </div>
    );
  }
  const sign = line.kind === "add" ? "+" : line.kind === "del" ? "-" : "";
  const cls = line.kind === "add" ? "dl a" : line.kind === "del" ? "dl d" : "dl";
  return (
    <div
      className={both ? `${cls} two` : cls}
      data-kind={line.kind}
      {...(line.cut === undefined ? {} : { "data-cut": line.cut })}
    >
      {both ? (
        <>
          <span className="n" data-side="old">
            {line.oldNo ?? ""}
          </span>
          <span className="n" data-side="new">
            {line.newNo ?? ""}
          </span>
        </>
      ) : (
        <span className="n">{line.newNo ?? line.oldNo ?? ""}</span>
      )}
      <span className="s">{sign}</span>
      <span className="c">{line.cut === undefined ? line.text : `${line.text}…`}</span>
    </div>
  );
}

/** Renders one file's diff. `id` keys its "show all" state. */
export function DiffView({
  diff,
  id,
  headerActions,
  both = false,
  footer,
}: {
  diff: FileDiff;
  id: string;
  headerActions?: ReactNode;
  /** Old and new line numbers in two columns (the approval card's preview of an edit). */
  both?: boolean;
  /** Shown under the lines (the preview's count of lines the core left out). */
  footer?: ReactNode;
}): ReactElement {
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
      data-testid="transcript.diff-view-div-surface-1"
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
        {headerActions}
      </div>
      {lines.map((line, i) => (
        <DiffLineRow key={i} line={line} both={both} />
      ))}
      {hidden > 0 ? (
        <Button
          data-testid="transcript.diff-view-toggle"
          type="button"
          appearance="expander"
          aria-label="Show full diff"
          title="Show full diff"
          onClick={() => {
            transcript.toggle(key, true);
          }}
        >
          Show all ({String(hidden)} more lines)
        </Button>
      ) : null}
      {footer}
    </div>
  );
}
