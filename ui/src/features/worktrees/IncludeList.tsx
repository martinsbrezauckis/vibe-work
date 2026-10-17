/**
 * What a `.worktreeinclude` file copies into a new copy: the one-line summary ("Copies 3 files from
 * `.worktreeinclude`" before the copy, "Copied 3 files..." after) with the file names and the skipped ones (and
 * why) in an expandable list. Nothing renders for a repository without that file.
 * Depends on: model.ts. Called by: CopySection (the plan), IncludeNote (the result).
 * Invariants: names only (the core never sends contents); names are text, never markup; the list is keyboard
 * reachable (a native details element).
 */
import type { ReactElement } from "react";

import type { IncludeReport } from "../../protocol/core";
import { includeLine, skipText } from "./model";

/** The summary and its list. `plan` true before the copy exists. */
export function IncludeList({
  report,
  plan,
}: {
  report: IncludeReport;
  plan: boolean;
}): ReactElement | null {
  const line = includeLine(report, plan);
  if (line === "") return null;
  const extra = report.skippedTotal - report.skipped.length;
  const hasList = report.files.length > 0 || report.skipped.length > 0;
  return (
    <div className="wt-include" data-worktree="include">
      {hasList ? (
        <details>
          <summary
            data-testid="worktrees.include-list-summary"
            title="Show or hide file copy details"
            aria-label="Show or hide file copy details"
          >
            {line}
          </summary>
          <ul className="wt-files mono">
            {report.files.map((f) => (
              <li key={`f-${f.path}`}>{f.path}</li>
            ))}
            {report.skipped.map((s) => (
              <li key={`s-${s.path}`} className="wt-skipped">
                {s.path}: not copied, {skipText(s.reason)}
              </li>
            ))}
            {extra > 0 && <li className="wt-skipped">and {String(extra)} more not copied</li>}
          </ul>
        </details>
      ) : (
        <p className="copy-reason">{line}</p>
      )}
      {report.stopped !== null && (
        <p className="copy-reason" role="note">
          {report.stopped}
        </p>
      )}
    </div>
  );
}
