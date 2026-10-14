/**
 * Bundles and sends the handbook skill's user requests from the handbook tab and Projects overview.
 * Depends on: the prompt files beside the skill; composer, shell and toast controllers.
 * Called by: usePagesTab and state/projects. Requests leave drafts and attachments untouched.
 */
import createHandbook from "../../../../app/plugin/vw/skills/handbook/create-handbook.md?raw";
import writePlan from "../../../../app/plugin/vw/skills/handbook/write-plan.md?raw";
import { toCallError } from "../../protocol/client/types";
import { runningTurnId } from "../composer/model/session";
import type { Composer } from "../../state/composer";
import { computeLayout } from "../../state/panelLayout";
import type { App } from "../shell/AppContext";

/** The handbook request the user chose. */
export type HandbookPrompt = "create" | "plan";

/** Why handbook requests are unavailable during a turn or a send in flight. */
export const HANDBOOK_BUSY_TOOLTIP = "The agent is working; try when it is done";

/** Reads the composer's running-turn predicate, including turns whose session status has not caught up. */
export function handbookRequestBusy(composer: Composer, sessionId: string): boolean {
  const entry = composer.core?.store.get().sessions[sessionId];
  return (
    composer.store.get().sending[sessionId] === true ||
    runningTurnId(entry?.state ?? null, entry?.session) !== null
  );
}

/** Reads the bundled request, using the project's configured handbook folder. */
export function handbookPrompt(kind: HandbookPrompt, folder = "docs/handbook"): string {
  return (kind === "create" ? createHandbook : writePlan)
    .trim()
    .replaceAll("docs/handbook", folder);
}

/**
 * Sends the prompt as the user's message through the composer, separately from its draft.
 * A drawer or maximized panel closes to reveal the transcript; a failed send leaves no prompt draft.
 * Only a trusted native button event preserves window origin through the deferred send.
 */
export async function sendHandbookPrompt(
  app: Pick<App, "composer" | "shell" | "toasts">,
  sessionId: string,
  kind: HandbookPrompt,
  folder = "docs/handbook",
  project?: string,
  trustedClick = false,
): Promise<void> {
  if (app.composer.actions === null) return;
  if (handbookRequestBusy(app.composer, sessionId)) return;
  const prompt = handbookPrompt(kind, project === undefined ? folder : `${project}/${folder}`);
  app.shell.actions.selectSession(sessionId);
  const shell = app.shell.store.get();
  if (computeLayout(shell, window.innerWidth).drawer || shell.maximized)
    app.shell.actions.closePanel();
  try {
    const sent = await app.composer.actions.sendMessage(sessionId, prompt, trustedClick);
    if (!sent.ok && sent.kind !== "key")
      app.toasts.push({ kind: "warn", bellKind: "failure", text: sent.message });
  } catch (thrown) {
    app.toasts.push({ kind: "warn", bellKind: "failure", text: toCallError(thrown).error.message });
  }
}
