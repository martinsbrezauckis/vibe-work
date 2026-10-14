/**
 * The Debug API side of Handbook Format 1.2 (every feature reachable, acting as the user):
 * the `ui/project` actions for page inputs, the visual check and "Share as one file", and the `ui/plan` route of the
 * plan checklist.
 * `ui/project` actions (the selected session; `path` defaults to the shown page):
 * - `answers {path?}`: the inputs with the user's answers (every page with inputs when nothing is shown);
 * - `answer {path?, input, value}` (`value` as the core's `AnswerValue`, `null` clears), `sendAnswers {path?}`: every
 *   answer of the page into the composer as ONE chip (Debug API facade), then marked sent;
 * - `check {path?}`: the visual check now; `export {path?, destination?}`: the file's text, or written to
 *   `destination` (the Debug API's equivalent of the native save dialog, like its `pickFolder`).
 * `ui/plan` actions: `state` (default): the plan checklist (path, title, steps, progress); `open {step}`: what a click
 * on a step does (the Project tab at that step of the plan page).
 * Depends on: state/uiControl, store.ts, inputs.ts, point.ts, model.ts, AppContext types. Called by: pagesRoutes.ts,
 * register.tsx.
 * Invariants: bodies are validated (`invalidParams`); chips are data, never commands; nothing here writes a page.
 */
import type { AnswerValue } from "../../protocol/core";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { answersChip } from "./inputs";
import { planProgress } from "./model";
import { insertChip, sessionChipFolder } from "./point";
import type { Pages } from "./store";

const ACTIONS = ["answers", "answer", "sendAnswers", "check", "export"];

/** True when `action` is one of this file's `ui/project` actions. */
export function isHandbookAction(action: unknown): action is string {
  return typeof action === "string" && ACTIONS.includes(action);
}

function pathOf(pages: Pages, sessionId: string, input: Record<string, unknown>): string {
  const given = input["path"];
  if (typeof given === "string" && given !== "") return given;
  const shown = pages.store.get().hubs[sessionId]?.viewer?.path ?? null;
  if (shown === null)
    throw new UiControlError("invalidParams", "path is required (no page is shown)");
  return shown;
}

function value(raw: unknown): AnswerValue | null {
  if (raw === null) return null;
  if (
    typeof raw !== "object" ||
    Array.isArray(raw) ||
    typeof (raw as { type?: unknown }).type !== "string"
  )
    throw new UiControlError("invalidParams", "value must be an answer ({type, ...}) or null");
  return raw as AnswerValue;
}

/** One `ui/project` action for `sessionId` (see the file header). */
export async function handbookAction(
  app: App,
  pages: Pages,
  sessionId: string,
  action: string,
  input: Record<string, unknown>,
): Promise<unknown> {
  switch (action) {
    case "answers": {
      const given = input["path"];
      const shown = pages.store.get().hubs[sessionId]?.viewer?.path ?? undefined;
      const path = typeof given === "string" && given !== "" ? given : shown;
      return pages.answersFor(sessionId, path ?? undefined);
    }
    case "answer": {
      const path = pathOf(pages, sessionId, input);
      const id = input["input"];
      if (typeof id !== "string" || id === "")
        throw new UiControlError("invalidParams", "input must be an input id");
      await pages.answer(sessionId, path, id, value(input["value"]));
      return {
        error: pages.store.get().error[sessionId] ?? null,
        answers: await pages.answersFor(sessionId, path),
      };
    }
    case "sendAnswers": {
      const path = pathOf(pages, sessionId, input);
      const result = await pages.answersFor(sessionId, path);
      const page = result?.type === "answers" ? result.pages[0] : undefined;
      if (page === undefined) throw new UiControlError("invalidParams", "the page has no inputs");
      const chip = answersChip(sessionChipFolder(app, pages, sessionId), page);
      const outcome = insertChip(app, sessionId, chip, "debug-api");
      if (!outcome.ok) throw new UiControlError("invalidParams", outcome.reason);
      await pages.markAnswersSent(sessionId, path);
      return { chip: chip.label, text: chip.text };
    }
    case "check":
      return pages.visualCheck(sessionId, pathOf(pages, sessionId, input));
    case "export": {
      const destination = input["destination"];
      if (destination !== undefined && typeof destination !== "string")
        throw new UiControlError("invalidParams", "destination must be an absolute file path");
      const result = await pages.exportPage(
        sessionId,
        pathOf(pages, sessionId, input),
        destination,
      );
      return result ?? { error: pages.store.get().error[sessionId] ?? null };
    }
    default:
      throw new UiControlError("invalidParams", `unknown action ${action}`);
  }
}

/** Registers `ui/plan`; returns the function that removes it. */
export function registerPlanRoute(
  app: App,
  pages: Pages,
  openStep: (sessionId: string, path: string, step: string) => void,
): () => void {
  return app.uiControl.register("plan", async (body: unknown) => {
    const input = (body ?? {}) as Record<string, unknown>;
    const sessionId = app.shell.store.get().selected;
    if (sessionId === null) throw new UiControlError("invalidParams", "no session is shown");
    await pages.loadPlan(sessionId);
    const plan = pages.store.get().plans[sessionId];
    const action = input["action"] ?? "state";
    if (action === "open") {
      const step = input["step"];
      if (
        plan?.path == null ||
        typeof step !== "string" ||
        !plan.facts?.steps.some((s) => s.id === step)
      )
        throw new UiControlError("invalidParams", "step must be a step id of the current plan");
      openStep(sessionId, plan.path, step);
    } else if (action !== "state") {
      throw new UiControlError("invalidParams", "action must be state or open");
    }
    const steps = plan?.facts?.steps ?? [];
    return {
      sessionId,
      path: plan?.path ?? null,
      title: plan?.facts?.title ?? null,
      steps,
      progress: planProgress(steps),
      tabOpen: app.shell.store.get().panels[sessionId]?.tabs.includes("plan") ?? false,
    };
  });
}
