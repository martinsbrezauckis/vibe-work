/**
 * Minimal Debug API input/read transport while the shell has failed. Reuses the secret-neutral
 * control reader and input routes; called only by the recovery view, removed before shell remount.
 * Failure of diagnostic wiring must leave physical recovery controls usable.
 */
import { runUiControl } from "../../state/uiControl";
import type { App } from "./AppContext";
import { registerControlRead } from "./controlRead";
import { registerInputRoutes } from "./inputRoutes";
import { feedbackFor } from "../../state/feedback";
import { screenshotSecrets } from "./screenshotSecrets";
import { UiControlError } from "../../state/uiControl";

/** Installs only the controls needed to inspect and drive the recovery screen. */
export function startRecoveryWiring(app: App): () => void {
  const abort = new AbortController();
  const stops: (() => void)[] = [];
  try {
    stops.push(registerControlRead(app));
    stops.push(registerInputRoutes(app));
    stops.push(
      app.uiControl.register(
        "secrets",
        (body) => {
          if (typeof body !== "object" || body === null || Array.isArray(body))
            throw new UiControlError("invalidParams", "the body must be a JSON object");
          return screenshotSecrets((body as Record<string, unknown>)["lock"]);
        },
        () => true,
      ),
    );
    const core = app.core;
    if (core !== null) {
      const active = () =>
        core.client.kind !== "tauri" ||
        core.store.get().debugApi.enabled ||
        feedbackFor(core).store.get().capturing;
      void runUiControl(core.client.uiControl, app.uiControl, active, abort.signal).catch(
        console.error,
      );
    }
  } catch (error) {
    console.error(error);
  }
  return () => {
    abort.abort();
    for (const stop of stops.reverse()) stop();
  };
}
