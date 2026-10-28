/**
 * The Project tab's wiring for one session: what the tab shows (hub or page, plan steps, comments, the
 * viewer) and the callbacks its parts call. Every decision is in store.ts, model.ts or point.ts; this hook only
 * connects them to the selected session and composer.
 * Depends on: store.ts, model.ts, point.ts, shell hooks, state/sessionSummary, transcript todos. Called by: PagesTab.
 * Invariants: comments and points end as context chips (never commands); "Send to Vibe" marks the comment sent only
 * after the chip is in the composer; when the tab shows another folder's handbook than the session's own (such as its project's main folder) the chips name it by its absolute path and `foreign` says so.
 */
import { writeClipboard } from "../../state/clipboard";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { AnswerValue, Comment, JsonValue, PickedSection, Rect } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { useStore } from "../../state/createStore";
import { statusType } from "../../state/sessionSummary";
import { useApp } from "../shell/AppContext";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useCoreState } from "../shell/hooks";
import { latestTodos } from "../transcript/model/todos";
import {
  type PlanMismatch,
  chipFolder,
  commentChip,
  fixChip,
  mismatchChip,
  sectionChip,
} from "./model";
import { usePageMenu } from "./PageMenu";
import { answersChip } from "./inputs";
import { insertChip, pointSection } from "./point";
import type { Pages } from "./store";
import { handbookRequestBusy, sendHandbookPrompt } from "./handbookPrompts";

const NONE: readonly JsonValue[] = [];

/** The tab's view model and actions for `sessionId`. */
export function usePagesTab(pages: Pages, sessionId: string) {
  const app = useApp();
  const key = useKeyReadiness();
  const keyBlocked = key !== null;
  const hub = useStore(pages.store, (s) => s.hubs[sessionId]);
  const page = useStore(pages.store, (s) => s.pages[sessionId] ?? null);
  const hubView = useStore(pages.store, (s) => s.hubView[sessionId] === true);
  const busy = useStore(pages.store, (s) => s.busy[sessionId] === true);
  const picking = useStore(pages.store, (s) => s.picking[sessionId] === true);
  const error = useStore(pages.store, (s) => s.error[sessionId] ?? null);
  const planNotice = useStore(pages.store, (s) => s.notice[sessionId] ?? null);
  const answers = useStore(pages.store, (s) => s.answers[sessionId] ?? null);
  const checking = useStore(pages.store, (s) => s.checking[sessionId] === true);
  const entry = useCoreState((s) => s.sessions[sessionId]);
  useStore(app.composer.store, (s) => s.sending[sessionId]);
  const [draft, setDraft] = useState<PickedSection | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const history = Array.isArray(entry?.state?.["history"]) ? entry.state["history"] : NONE;
  const todos = useMemo(() => latestTodos(history), [history]);
  const status = entry === undefined ? "idle" : statusType(entry.session);
  const running = status === "running" || status === "blocked";

  useEffect(() => {
    void pages.load(sessionId);
  }, [pages, sessionId]);

  const viewer = hub?.viewer ?? null;
  const path = viewer?.path ?? null;
  const showingPage = path !== null && !hubView;
  const folder = hub?.settings.folder ?? "docs/handbook";
  const cwd = typeof entry?.session["cwd"] === "string" ? entry.session["cwd"] : null;
  // the folder chips name pages by (absolute when the handbook is the project's main folder's).
  const chips = chipFolder(hub, cwd);
  const foreign =
    chips === folder || hub === undefined ? null : { name: hub.name, path: hub.project };
  const note = (text: string) => {
    setNotice(text);
  };
  const chipIn = (chip: Parameters<typeof insertChip>[2]) => {
    const outcome = insertChip(app, sessionId, chip);
    if (outcome.ok) note(`Added to your message: ${chip.label}`);
    return outcome.ok;
  };

  const send = (comment: Comment) => {
    if (chipIn(commentChip(chips, comment)))
      void pages.markSent(sessionId, comment.page, [comment.id]);
  };
  // The page area last shown: the page menu opens at a right click's point in it (the page reports page points).
  const area = useRef<Rect | null>(null);
  const report = useCallback(
    (rect: Rect | null) => {
      if (rect !== null) area.current = rect;
      pages.layout(rect === null ? null : { sessionId, rect });
    },
    [pages, sessionId],
  );
  const menu = usePageMenu(pages, sessionId, showingPage, area);
  const shownPath = showingPage ? path : null;
  const answer = useCallback(
    (input: string, value: AnswerValue | null) => {
      if (shownPath !== null) void pages.answer(sessionId, shownPath, input, value);
    },
    [pages, sessionId, shownPath],
  );
  return {
    hub,
    page,
    /**: the shown page's inputs with the user's answers (`null`: none). */
    answers: shownPath === null ? null : answers,
    /**: the shown page is a prototype (runs its own script, sandboxed). */
    prototype: page?.facts.kind === "prototype",
    /**: the page's right-click menu (open while `menu.at` is set; the page is parked under it). */
    menu: {
      ...menu,
      point: () =>
        void pages.menuSection(sessionId).then((picked) => {
          if (picked === null) return;
          const outcome = insertChip(app, sessionId, sectionChip(chips, picked));
          note(outcome.ok ? `Added to your message: ${outcome.chip.label}` : outcome.reason);
        }),
      comment: () =>
        void pages.menuSection(sessionId).then((picked) => {
          if (picked !== null) setDraft(picked);
        }),
      copy: () => {
        const copying = pages.menuCopy(sessionId).then(async (text) => {
          if (text === null) return;
          try {
            await writeClipboard(text);
            app.toasts.feedback("Copied the selected text.");
          } catch {
            note("The selected text could not be copied.");
          }
        });
        // The native selection read must finish before the UI-control request lets go of its clipboard writer.
        void app.core?.trackAction(copying);
      },
    },
    /**: the shown page's current visual check report. */
    visual: shownPath === null ? undefined : hub?.visual[shownPath],
    checking,
    busy,
    picking,
    error,
    notice: notice ?? planNotice,
    draft,
    todos,
    running,
    requestBusy: keyBlocked || handbookRequestBusy(app.composer, sessionId),
    requestTooltip: key?.message,
    viewer,
    path: showingPage ? path : null,
    showingPage,
    folder,
    /** The handbook shown is the project's main folder's, not the session's own folder. */
    foreign,
    report,
    open: (p: string) => void pages.open(sessionId, p),
    /** Sends the handbook request in this project's current chat. */
    askCreate: (trustedClick: boolean) => {
      void sendHandbookPrompt(app, sessionId, "create", folder, foreign?.path, trustedClick);
    },
    /** Sends the first plan request separately from the user's draft. */
    askPlan: (trustedClick: boolean) => {
      void sendHandbookPrompt(app, sessionId, "plan", folder, foreign?.path, trustedClick);
    },
    /** Puts "fix these findings" into the composer as a chip (the user sends it). */
    askFix: () => {
      const findings = hub?.hub?.validation.findings ?? [];
      chipIn(fixChip(chips, findings));
    },

    toolbar: {
      hub: () => {
        pages.showHub(sessionId);
      },
      back: () => void pages.back(sessionId),
      forward: () => void pages.forward(sessionId),
      reload: () => void pages.reload(sessionId),
      plan: () =>
        void pages.openPlan(sessionId).then((ok) => {
          if (!ok) note("This project has no plan page yet.");
        }),
      comment: () =>
        void pages.pickSection(sessionId).then((picked) => {
          setDraft(picked);
        }),
      point: () =>
        void pointSection(app, pages, sessionId).then((outcome) => {
          note(outcome.ok ? `Added to your message: ${outcome.chip.label}` : outcome.reason);
        }),
      cancelPick: () => void pages.cancelPick(),
    },
    comments: {
      save: async (text: string, andSend: boolean): Promise<boolean> => {
        if (draft === null) return false;
        const target = {
          path: draft.page,
          section: draft.section,
          text,
          ...(draft.quote === null ? {} : { quote: draft.quote }),
        };
        const id = await pages.addComment(sessionId, target);
        if (id === null) return false;
        setDraft(null);
        const stored = pages.store.get().pages[sessionId]?.attached.find((c) => c.id === id);
        if (andSend && stored !== undefined) send(stored);
        return true;
      },
      cancel: () => {
        setDraft(null);
      },
      send,
      remove: (comment: Comment) => void pages.deleteComment(sessionId, comment.page, comment.id),
    },
    /**: sets or clears the answer to one input of the shown page. */
    answer,
    /**: every answer of the shown page into the composer as one chip, then marked sent. */
    sendAnswers: () => {
      if (answers === null || shownPath === null) return;
      if (chipIn(answersChip(chips, answers))) void pages.markAnswersSent(sessionId, shownPath);
    },
    /**: runs the visual check of the shown page now. */
    check: () => {
      if (shownPath !== null) void pages.visualCheck(sessionId, shownPath);
    },
    /**: "Share as one file" (the native save dialog). */
    share: () => {
      if (shownPath === null) return;
      void app.core?.client
        .exportPage(sessionId, shownPath)
        .then((result) => {
          if (result?.type === "exported" && result.savedTo !== null)
            note(`Saved as one file: ${result.savedTo}`);
        })
        .catch((thrown: unknown) => {
          note(toCallError(thrown).error.message);
        });
    },
    askUpdate: (mismatch: PlanMismatch) => {
      if (path !== null) chipIn(mismatchChip(chips, path, mismatch));
    },
    openExternal: (url: string) => {
      app.transcript.askLink(url);
      void pages.clearExternal(sessionId);
    },
    dismissExternal: () => void pages.clearExternal(sessionId),
  };
}
