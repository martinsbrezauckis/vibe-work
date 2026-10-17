/**
 * The project handbook's live hub (Handbook Format 1.1 §8, §10): the manifest (project name, summary,
 * status), notices for a missing or stale index and for a handbook in the earlier format (each with a button that
 * puts a request for Vibe into the composer), the validator's findings, and the pages in index order: roadmap and
 * changelog, one group per kind, Other documents, Archive (folded). Markdown pages show in the same list with a tag.
 * Depends on: store.ts (answers), protocol types, HubRows. Called by: PagesTab.
 * Invariants: renders only; every page text (titles, statuses, descriptions, finding messages) is plain text; the
 * creation and first-plan requests send user messages; other notices prepare context chips. No action writes files.
 */
import { type ReactElement, useState } from "react";

import type { Finding, Hub as HubData } from "../../protocol/core";
import { Findings, Groups } from "./HubRows";
import type { HubAnswer } from "./store";
import { HANDBOOK_BUSY_TOOLTIP } from "./handbookPrompts";

const INDEX_TEXT: Record<HubData["index"], string | null> = {
  current: null,
  missing:
    "This handbook has no index.html yet: the list below is built by Vibe Work from the page heads and not saved.",
  notManifest: "index.html is not a handbook manifest (its handbook.kind is not index).",
  stale:
    "index.html is out of date: pages are missing from it or it is older than the newest page.",
  preFormat: "index.html was written by an earlier build of Vibe Work and needs to be replaced.",
};

function counts(errors: number, warnings: number): string {
  const part = (n: number, word: string) => `${String(n)} ${word}${n === 1 ? "" : "s"}`;
  if (errors === 0 && warnings === 0) return "No errors or warnings";
  return [
    errors > 0 ? part(errors, "error") : null,
    warnings > 0 ? part(warnings, "warning") : null,
  ]
    .filter((x): x is string => x !== null)
    .join(" · ");
}

/** The hub of `answer` (the selected session's project). */
export function Hub({
  answer,
  open,
  askFix,
  askMigrate,
  askCreate,
  askPlan,
  requestBusy = false,
  requestTooltip = HANDBOOK_BUSY_TOOLTIP,
}: {
  answer: HubAnswer;
  open: (path: string) => void;
  askFix: () => void;
  askMigrate: () => void;
  askCreate: (trustedClick: boolean) => void;
  askPlan: (trustedClick: boolean) => void;
  requestBusy?: boolean;
  requestTooltip?: string;
}): ReactElement {
  const [showFindings, setShowFindings] = useState(false);
  const hub = answer.hub;
  if (hub === null) {
    return (
      <div className="pages-hub">
        <div className="pages-empty">
          <p>This project has no handbook yet to keep its plans, decisions and reports together.</p>
          <button
            data-testid="pages.hub-ask-create"
            type="button"
            className="pages-btn go"
            onClick={(event) => {
              askCreate(event.nativeEvent.isTrusted);
            }}
            disabled={requestBusy}
            title={requestBusy ? requestTooltip : undefined}
          >
            Ask Vibe to create the handbook
          </button>
        </div>
      </div>
    );
  }
  const manifest = hub.manifest;
  const v = hub.validation;
  const byPage = (path: string): Finding[] => v.findings.filter((f) => f.page === path);
  const bookFindings = v.findings.filter(
    (f) =>
      f.page === "index.html" || !hub.groups.some((g) => g.pages.some((p) => p.path === f.page)),
  );
  const indexText = INDEX_TEXT[hub.index];
  return (
    <div className="pages-hub">
      <div className="pages-hub-top">
        <h3>{manifest?.project ?? `${answer.name} handbook`}</h3>
        {manifest?.status != null && (
          <span className="pages-pill" data-status={manifest.status}>
            {manifest.status}
          </span>
        )}
      </div>
      {manifest?.description != null && <p className="pages-summary">{manifest.description}</p>}
      {!hub.truncated && !hub.groups.some((g) => g.pages.some((p) => p.kind === "plan")) && (
        <button
          data-testid="pages.hub-ask-plan"
          type="button"
          className="pages-plan-link"
          onClick={(event) => {
            askPlan(event.nativeEvent.isTrusted);
          }}
          disabled={requestBusy}
          title={requestBusy ? requestTooltip : undefined}
        >
          Ask Vibe to write a plan page
        </button>
      )}
      {hub.legacy && (
        <div className="server-note" role="status" data-notice="migrate">
          <span>
            This handbook uses the earlier format ({hub.folder}/, vw-* names). Vibe can move it to
            docs/handbook/ (Handbook Format 1.1) when you ask.
          </span>
          <button
            data-testid="pages.hub-pages-btn"
            type="button"
            className="pages-btn"
            onClick={askMigrate}
          >
            Ask Vibe to migrate it
          </button>
        </div>
      )}
      {indexText !== null && !hub.legacy && (
        <div className="server-note" role="status" data-notice={hub.index}>
          <span>{indexText}</span>
          <button
            data-testid="pages.hub-pages-btn-2"
            type="button"
            className="pages-btn"
            onClick={askFix}
          >
            Ask Vibe to fix the handbook
          </button>
        </div>
      )}
      <div className="pages-health" data-errors={v.errors} data-warnings={v.warnings}>
        <span>
          {counts(v.errors, v.warnings)}
          {v.infos > 0 && ` · ${String(v.infos)} hint${v.infos === 1 ? "" : "s"}`} ·{" "}
          {String(v.pages)} pages in {hub.folder}/
        </span>
        {v.findings.length > 0 && (
          <button
            data-testid="pages.hub-set-show-findings"
            type="button"
            className="pages-btn"
            aria-expanded={showFindings}
            onClick={() => {
              setShowFindings(!showFindings);
            }}
          >
            {showFindings ? "Hide findings" : "Show findings"}
          </button>
        )}
        {(v.errors > 0 || v.warnings > 0) && indexText === null && !hub.legacy && (
          <button
            data-testid="pages.hub-pages-btn-3"
            type="button"
            className="pages-btn"
            onClick={askFix}
          >
            Ask Vibe to fix them
          </button>
        )}
      </div>
      {showFindings && <Findings findings={bookFindings} label="Handbook findings" />}
      <Groups
        groups={hub.groups}
        counts={answer.commentCounts}
        findingsOf={byPage}
        open={open}
        visualOf={(path) => answer.visual[path]}
      />
    </div>
  );
}
