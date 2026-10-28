/**
 * Opens Settings › Allowed without asking: from Safety's Review link and from a
 * session's grants list ("See everything allowed without asking"), which opens that session's sub-group.
 * Depends on: AppContext types, allowed/store, allowed/entries. Called by: AllowedSummary (Review), the session
 * grants list (session grants), allowed/routes.
 */
import type { App } from "../../shell/AppContext";
import { topSession } from "./entries";
import { allowedFor, sessionInfos } from "./store";

/** The Settings category id of the page. */
const ALLOWED_CATEGORY = "allowed";

/**
 * Opens the page. With `sessionId`, "Answers in open sessions" and that session's sub-group open (a subagent's
 * session opens its parent's sub-group, where its answers are listed) and focus goes to that sub-group's header;
 * Find and Project are cleared so nothing hides it.
 */
export function openAllowed(app: App, options: { sessionId?: string } = {}): void {
  const allowed = allowedFor(app);
  if (options.sessionId !== undefined) {
    const sessions = sessionInfos(app.core?.store.get().sessions ?? {});
    allowed.reveal("sessions", `sessions/${topSession(options.sessionId, sessions)}`);
  }
  allowed.showCategory(ALLOWED_CATEGORY);
}
