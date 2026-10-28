/**
 * The Project tab body: the project handbook. The live hub (pages grouped by kind, the manifest,
 * the validator's findings, index notices) or the page viewer (the
 * isolated engine placed over a `NativeViewport`), the plan's progress header on plan pages, a notice when a page
 * tried to open a link outside the pages folder, the comments of the shown page, and a note when the
 * handbook is in the project's main folder rather than the session's folder. It also shows the visual check badge of
 * the shown page, the controls for its inputs (answers; every control or one line, as the window's shell state
 * says), "Share as one file", the prototype badge, and the page's right-click menu (the page is parked under it while
 * it is open).
 * Depends on: usePagesTab, PagesToolbar, Hub, PlanHeader, CommentPanel, PageMenu, components/NativeViewport, shell
 * hooks.
 * Called by: register.tsx (the panel tab body).
 * Invariants: renders only; agent page content never renders in this window (only in the engine); every page text
 * shown here (titles, labels, comments, links) is plain text.
 */
import { Button } from "../../components/Controls";
import { TEXT } from "../../vocabulary";
import type { ReactElement } from "react";

import { NativeViewport } from "../../components/NativeViewport";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";
import { KeyNotice } from "../shell/KeyNotice";
import { AnswersPanel } from "./AnswersPanel";
import { CommentPanel } from "./CommentPanel";
import { Hub } from "./Hub";
import { PageMenu } from "./PageMenu";
import { PagesToolbar } from "./PagesToolbar";
import { PlanHeader } from "./PlanHeader";
import type { Pages } from "./store";
import { usePagesTab } from "./usePagesTab";
import { VisualCheck } from "./VisualCheck";

function SessionPages({ pages, sessionId }: { pages: Pages; sessionId: string }): ReactElement {
  const tab = usePagesTab(pages, sessionId);
  const { shell } = useApp();
  const answersShown = useShellState((s) => s.answersShown);
  const facts = tab.showingPage ? (tab.page?.facts ?? null) : null;
  const isPlan =
    facts !== null &&
    facts.steps.length > 0 &&
    (facts.kind === "plan" || tab.path?.startsWith("plans/") === true);
  return (
    <div
      data-item-id={sessionId}
      className="pages-tab"
      data-view={tab.showingPage ? "page" : "hub"}
    >
      <PagesToolbar
        path={tab.path}
        folder={tab.folder}
        busy={tab.busy}
        picking={tab.picking}
        prototype={tab.prototype}
        actions={{ ...tab.toolbar, share: tab.share }}
      />
      <KeyNotice message="Handbook requests use your Mistral key so the agent can write pages and plans." />
      {tab.showingPage && (
        <VisualCheck report={tab.visual} checking={tab.checking} check={tab.check} />
      )}
      {tab.error !== null && (
        <p className="key-problem" role="alert">
          {tab.error}
        </p>
      )}
      {tab.notice !== null && (
        <p className="pages-note" role="status">
          {tab.notice}
        </p>
      )}
      {tab.foreign !== null && (
        <p className="pages-note" role="note" data-foreign-handbook="true">
          This is the handbook of the project&apos;s main folder, {tab.foreign.name} (
          <span className="pages-url">{tab.foreign.path}</span>). {TEXT.thisSession} works in
          another folder; what you point Vibe at names the main folder&apos;s files.
        </p>
      )}
      {tab.picking && (
        <p className="pages-note" role="status">
          Select text or click a part of the page. Escape cancels.
        </p>
      )}
      {tab.viewer?.external != null && (
        <div className="pages-external" role="status">
          <span>
            The page tried to open <span className="pages-url">{tab.viewer.external}</span>. Pages
            only open pages of this folder.
          </span>
          <Button
            data-testid="pages.pages-tab-open-external"
            type="button"
            appearance="page"
            onClick={() => {
              tab.openExternal(tab.viewer?.external ?? "");
            }}
          >
            Open in browser…
          </Button>
          <Button
            data-testid="pages.pages-tab-pages-btn"
            type="button"
            appearance="page"
            onClick={tab.dismissExternal}
          >
            Dismiss
          </Button>
        </div>
      )}
      {isPlan && (
        <PlanHeader
          title={facts.title ?? tab.path ?? "Plan"}
          steps={facts.steps}
          todos={tab.todos}
          running={tab.running}
          askUpdate={tab.askUpdate}
        />
      )}
      {!tab.showingPage &&
        (tab.hub === undefined ? (
          <div className="empty">Reading this project&apos;s handbook…</div>
        ) : (
          <Hub
            answer={tab.hub}
            open={tab.open}
            askFix={tab.askFix}
            askCreate={tab.askCreate}
            askPlan={tab.askPlan}
            requestBusy={tab.requestBusy}
            {...(tab.requestTooltip === undefined ? {} : { requestTooltip: tab.requestTooltip })}
          />
        ))}
      <NativeViewport
        report={tab.report}
        live={tab.showingPage && tab.menu.at === null}
        label="Project handbook page (isolated view)"
        className="pages-viewport"
      />
      {tab.menu.at !== null && tab.path !== null && (
        <PageMenu
          at={tab.menu.at}
          path={tab.path}
          onClose={tab.menu.close}
          point={tab.menu.point}
          comment={tab.menu.comment}
          copy={tab.menu.copy}
        />
      )}
      {tab.answers !== null && (
        <AnswersPanel
          page={tab.answers}
          answer={tab.answer}
          send={tab.sendAnswers}
          shown={answersShown}
          onShownChange={shell.actions.setAnswersShown}
        />
      )}
      {tab.showingPage && (
        <CommentPanel
          attached={tab.page?.attached ?? []}
          detached={tab.page?.detached ?? []}
          draft={tab.draft}
          {...tab.comments}
        />
      )}
    </div>
  );
}

/** The tab. */
export function PagesTab({ pages }: { pages: Pages }): ReactElement {
  const sessionId = useShellState((s) => s.selected);
  if (sessionId === null)
    return <div className="empty">Choose a session to see its project handbook.</div>;
  return <SessionPages key={sessionId} pages={pages} sessionId={sessionId} />;
}
