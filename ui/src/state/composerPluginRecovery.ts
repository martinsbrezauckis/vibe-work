/** Retry a held chat in place through the typed core; called by composerSession, never consumes its draft. */
import type { CoreController } from "./coreStore";
import type { ComposerStore } from "./composerStore";
import { toCallError } from "../protocol/client/types";
import { arr, obj, str } from "../features/transcript/model/entries";
import { runningTurnId } from "../features/composer/model/session";

export async function retryPluginRefresh(
  core: CoreController,
  composer: ComposerStore,
  sessionId: string,
): Promise<void> {
  if (composer.store.get().sending[sessionId] === true) return;
  composer.setSending(sessionId, true);
  try {
    const entry = core.store.get().sessions[sessionId];
    const state = entry?.state;
    const rawStatus = entry?.session["status"] ?? null;
    const status = str(rawStatus) ?? str(obj(rawStatus)?.["type"] ?? null);
    // An empty chat has no durable conversation to resume. Working chats only request the deferred refresh.
    const idle = status === "idle" || status === "failed";
    if (
      idle &&
      runningTurnId(state ?? null, entry?.session) === null &&
      arr(state?.["turns"]).length > 0 &&
      arr(state?.["activeCallbacks"]).length === 0 &&
      arr(obj(state?.["turnQueue"] ?? null)?.["items"]).length === 0 &&
      !arr(state?.["backgroundProcesses"]).some((p) => obj(p)?.["status"] === "running")
    ) {
      await core.run({ type: "resumeSession", sessionId });
    }
    const result = await core.run({ type: "refreshPlugins", sessionId });
    const pending =
      result.type === "reply" &&
      typeof result.result === "object" &&
      result.result !== null &&
      !Array.isArray(result.result) &&
      result.result["pending"] === true;
    composer.note(
      sessionId,
      pending
        ? "Plugins will refresh when this reply ends."
        : "Plugins refreshed. Your draft is ready to send.",
    );
    if (!pending) composer.pluginReloaded(sessionId);
  } catch (thrown) {
    composer.pluginReloadRefused(sessionId, toCallError(thrown).error.message);
  } finally {
    composer.setSending(sessionId, false);
  }
}
