/**
 * The `ui/git` UI-control route: an external agent drives the branch bar, the dialogs and
 * the PR card exactly like clicks do. While it is served, commands go out with the Debug API origin
 * (`state/coreStore.ts`), so the core's native confirmations are answered as the user's and prepared
 * messages become Debug-API-marked chips.
 * Depends on: state/uiControl, state/git, model.ts, AppContext types. Called by: changes/register.tsx. The Rust
 * host lists the route (`crates/vw-host/src/host.rs`).
 * Invariants: bodies are validated (`invalidParams`); `sessionId` defaults to the shown session; every action
 * answers the route's state afterwards.
 */
import { UiControlError } from "../../state/uiControl";
import type { Git } from "../../state/git";
import type { App } from "../shell/AppContext";
import { barShown } from "./model";

const ACTIONS =
  "state, reload, selectRepo, openCommit, setMessage, toggleFile, runHooks, commit, push, openPr, setPr, createPr, refreshCard, sendChecks, sendReview, openBring, bringIn, askVibe or close";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(value: unknown, name: string): string {
  if (typeof value !== "string")
    throw new UiControlError("invalidParams", `${name} must be a string`);
  return value;
}

/** The route's answer: what the bar, the open dialog and the card show. */
function state(git: Git, sessionId: string) {
  const s = git.store.get();
  const status = s.status[sessionId] ?? null;
  const dialog = s.dialog !== null && s.dialog.sessionId === sessionId ? s.dialog : null;
  return {
    sessionId,
    repo: s.selected[sessionId] ?? status?.repo ?? null,
    repos: status?.repos ?? [],
    shown: barShown(status ?? undefined),
    status,
    card: s.cards[sessionId] ?? null,
    busy: s.busy[sessionId] === true,
    notice: s.notice[sessionId] ?? null,
    output: s.output[sessionId] ?? null,
    dialog:
      dialog === null
        ? null
        : dialog.kind === "commit"
          ? {
              kind: "commit",
              branch: dialog.plan.branch,
              message: dialog.message,
              messageFrom: dialog.messageFrom,
              messageNote: dialog.messageNote,
              files: dialog.plan.files.map((f) => ({
                path: f.path,
                status: f.status,
                included: !dialog.excluded.includes(f.path),
              })),
              author: dialog.plan.author,
              authorProblem: dialog.plan.authorProblem,
              hooks: dialog.plan.hooks,
              runHooks: dialog.runHooks,
              credit: dialog.plan.credit,
              note: dialog.plan.note,
              blocker: dialog.plan.blocker,
            }
          : dialog.kind === "pr"
            ? {
                kind: "pr",
                plan: dialog.plan,
                title: dialog.title,
                body: dialog.body,
                base: dialog.base,
                draft: dialog.draft,
              }
            : { kind: "bring", plan: dialog.plan },
  };
}

/** Registers `ui/git`; returns the function that removes it. */
export function registerGitRoute(app: App, git: Git): () => void {
  return app.uiControl.register("git", async (body) => {
    const params = asObject(body);
    const sessionId =
      params["sessionId"] === undefined
        ? app.shell.store.get().selected
        : text(params["sessionId"], "sessionId");
    if (sessionId === null) throw new UiControlError("invalidParams", "no session is shown");
    const action = params["action"] ?? "state";
    if (git.store.get().status[sessionId] === undefined) await git.load(sessionId);
    if (params["repo"] !== undefined) {
      const repo = text(params["repo"], "repo");
      await git.selectRepo(sessionId, repo);
      const current = git.store.get();
      if (current.selected[sessionId] !== repo || current.status[sessionId]?.repo !== repo)
        throw new UiControlError("invalidParams", "repo must be a discovered repository root");
    }
    switch (action) {
      case "state":
        break;
      case "reload":
        await git.load(sessionId);
        await git.card(sessionId, false);
        break;
      case "selectRepo":
        break;
      case "openCommit":
        await git.openCommit(sessionId);
        break;
      case "setMessage":
        git.editCommit({ message: text(params["text"], "text") });
        break;
      case "toggleFile": {
        const path = text(params["path"], "path");
        const d = git.store.get().dialog;
        if (d?.kind !== "commit")
          throw new UiControlError("invalidParams", "the Commit dialog is not open");
        const on = params["on"] !== false;
        git.editCommit({
          excluded: on ? d.excluded.filter((p) => p !== path) : [...d.excluded, path],
        });
        break;
      }
      case "runHooks":
        git.editCommit({ runHooks: params["on"] !== false });
        break;
      case "commit": {
        const then = params["then"] ?? "none";
        if (then !== "none" && then !== "push" && then !== "pr")
          throw new UiControlError("invalidParams", "then must be none, push or pr");
        await git.commit(then);
        break;
      }
      case "push":
        await git.push(
          sessionId,
          params["remote"] === undefined ? undefined : text(params["remote"], "remote"),
        );
        break;
      case "openPr":
        await git.openPr(sessionId);
        break;
      case "setPr": {
        const change: { title?: string; body?: string; base?: string; draft?: boolean } = {};
        if (params["title"] !== undefined) change.title = text(params["title"], "title");
        if (params["body"] !== undefined) change.body = text(params["body"], "body");
        if (params["base"] !== undefined) change.base = text(params["base"], "base");
        if (params["draft"] !== undefined) change.draft = params["draft"] === true;
        git.editPr(change);
        break;
      }
      case "createPr":
        await git.createPr();
        break;
      case "refreshCard":
        await git.card(sessionId, true);
        break;
      case "sendChecks":
        await git.prepare(sessionId, "checks", "debug-api");
        break;
      case "sendReview":
        await git.prepare(sessionId, "review", "debug-api");
        break;
      case "openBring":
        await git.openBring(sessionId);
        break;
      case "bringIn":
        await git.bringIn();
        break;
      case "askVibe":
        await git.prepare(sessionId, "updateFromBase", "debug-api");
        git.close();
        break;
      case "close":
        git.close();
        break;
      default:
        throw new UiControlError("invalidParams", `action must be ${ACTIONS}`);
    }
    return state(git, sessionId);
  });
}
