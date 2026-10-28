/**
 * The visual check of the shown page ("Visual check of agent-written pages"): a small badge (looks fine,
 * N issues, not checked yet, checking) that folds out the list of what the check found at desktop and phone width in
 * light and dark, and a button to check again now. The agent gets the same report through `handbook_visual_check`.
 * Depends on: protocol types. Called by: PagesTab; `VisualBadge` also by HubRows.
 * Invariants: renders only; element paths, texts and details are untrusted page text shown as plain text.
 */
import { Badge, Button } from "../../components/Controls";
import type { ReactElement } from "react";

import type { VisualReport } from "../../protocol/core";

const KIND: Readonly<Record<string, string>> = {
  cutOff: "Cut off",
  sideways: "Sideways scroll",
  contrast: "Hard to read",
  theme: "Ignores the theme",
  scriptError: "Script error",
};

/** The badge's words and tone for `report`. */
function visualSummary(report: VisualReport | undefined): { text: string; tone: string } {
  if (report === undefined) return { text: "Not checked yet", tone: "none" };
  if (report.error !== null) return { text: "Check failed", tone: "warn" };
  const n = report.issues.length;
  if (n === 0 && report.notes.length === 0) return { text: "Looks fine", tone: "ok" };
  return { text: `${String(n)} look issue${n === 1 ? "" : "s"}`, tone: "warn" };
}

/** A small badge (hub rows, the page). */
export function VisualBadge({ report }: { report: VisualReport | undefined }): ReactElement | null {
  if (report === undefined) return null;
  const { text, tone } = visualSummary(report);
  return (
    <Badge
      presentation="page-check"
      data-tone={tone}
      title="Visual check at 1280 and 390 px, light and dark"
    >
      {text}
    </Badge>
  );
}

/** The page's visual check: badge, issues, "Check again". */
export function VisualCheck({
  report,
  checking,
  check,
}: {
  report: VisualReport | undefined;
  checking: boolean;
  check: () => void;
}): ReactElement {
  const { text, tone } = checking ? { text: "Checking…", tone: "none" } : visualSummary(report);
  const issues = report?.issues ?? [];
  return (
    <details className="pages-visual" data-tone={tone}>
      <summary data-testid="pages.visual-check-summary">
        <Badge presentation="page-check" data-tone={tone}>
          {text}
        </Badge>
        <span className="pages-note">desktop and phone width, light and dark</span>
      </summary>
      {report?.error != null && (
        <p className="key-problem" role="alert">
          {report.error}
        </p>
      )}
      {issues.length > 0 && (
        <ul className="pages-visual-list">
          {issues.map((issue, i) => (
            <li key={`${issue.kind}-${String(i)}`} data-kind={issue.kind}>
              <b>{KIND[issue.kind] ?? issue.kind}</b>
              <span className="pages-visual-where">{issue.views.join(", ")}</span>
              <span>
                {issue.element !== "" && <code>{issue.element}</code>} {issue.detail}
              </span>
            </li>
          ))}
        </ul>
      )}
      {report?.notes.map((note) => (
        <p key={note} className="pages-note">
          {note}
        </p>
      ))}
      <Button
        data-testid="pages.visual-check-pages-btn"
        type="button"
        appearance="page"
        disabled={checking}
        onClick={check}
      >
        Check again
      </Button>
    </details>
  );
}
