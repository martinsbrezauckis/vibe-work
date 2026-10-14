/**
 * The `ui/updates` UI-control route. The Rust side lists the route name (`crates/vw-host/src/host.rs`
 * SHELL_ROUTES). Every action calls the same controller function a click calls, so the route can never do
 * something the window cannot: while the request is served the controller marks the call with the Debug API
 * origin (logged; the request is the user's agreement, no dialog).
 * - `read` (default): the view as the host reports it, whether this build has a channel, and the notice text;
 * - `check`: check now; `download`: download the offered version (ask-before-downloading mode); `install`:
 *   install and restart (only a `ready` update; the dialog decides);
 * - `autoDownload {enabled}`: the Settings switch; `later`: hide the notice.
 * Depends on: AppContext, state/updates, state/uiControl. Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); there is no action that installs without the host's
 * agreement step; a declined dialog is a `confirmationDeclined` reply and changes nothing.
 */
import { toCallError } from "../../protocol/client/types";
import { UiControlError } from "../../state/uiControl";
import { noticeFor, statusLine } from "../../state/updates";
import type { App } from "../shell/AppContext";

function read(app: App) {
  const state = app.updates?.store.get();
  const view = state?.view ?? null;
  const notice = view === null ? null : noticeFor(view);
  return {
    channel: state?.available ?? false,
    view,
    status: view === null ? null : statusLine(view),
    notice:
      notice === null
        ? null
        : { headline: notice.headline, detail: notice.detail, actions: notice.actions },
    dismissed: state?.dismissed ?? null,
  };
}

/** Registers the route; returns the function that removes it. */
export function registerUpdatesRoutes(app: App): () => void {
  return app.uiControl.register("updates", async (body) => {
    const input =
      typeof body === "object" && body !== null && !Array.isArray(body)
        ? (body as Record<string, unknown>)
        : {};
    const updates = app.updates;
    const action = input["action"] ?? "read";
    if (updates === null || (!updates.store.get().available && action !== "read"))
      throw new UiControlError("denied", "this build has no update channel");
    try {
      switch (action) {
        case "read":
          break;
        case "check":
          await updates.check();
          break;
        case "download":
          await updates.download();
          break;
        case "install":
          await updates.install();
          break;
        case "autoDownload": {
          const enabled = input["enabled"];
          if (typeof enabled !== "boolean")
            throw new UiControlError("invalidParams", "enabled must be true or false");
          await updates.setAutoDownload(enabled);
          break;
        }
        case "later":
          updates.dismiss();
          break;
        default:
          throw new UiControlError(
            "invalidParams",
            "action must be read, check, download, install, autoDownload or later",
          );
      }
    } catch (thrown) {
      if (thrown instanceof UiControlError) throw thrown;
      const error = toCallError(thrown).error;
      throw new UiControlError(error.code, error.message);
    }
    return read(app);
  });
}
