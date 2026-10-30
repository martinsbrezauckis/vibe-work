/**
 * The `ui/models` UI-control route (the default-model card). The Rust side lists the route name (`crates/vw-host/src/host.rs`
 * SHELL_ROUTES). Each action calls the same controller function the card's buttons call:
 * - `read` (default): `{open, gone {alias, label} | null, models [{alias, label}], saving, error}`;
 * - `check`: runs the launch check again; `pick {alias}`: saves that model as the default and closes the card;
 *   `later`: closes the card.
 * Depends on: AppContext, state/uiControl, state/routeInput. Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); `pick` takes only a model the card offers.
 */
import { bodyObject } from "../../state/routeInput";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";

function read(app: App) {
  const state = app.defaultModel?.store.get();
  return {
    open: state?.gone !== null && state?.gone !== undefined,
    gone: state?.gone ?? null,
    models: state?.models ?? [],
    saving: state?.saving ?? false,
    error: state?.error ?? null,
  };
}

/** Registers the route; returns the function that removes it. */
export function registerDefaultModelRoutes(app: App): () => void {
  return app.uiControl.register("models", async (body) => {
    const input = bodyObject(body);
    const controller = app.defaultModel;
    if (controller === null) throw new UiControlError("serverUnavailable", "the app has no core");
    switch (input["action"] ?? "read") {
      case "read":
        break;
      case "check":
        await controller.check();
        break;
      case "pick": {
        const alias = input["alias"];
        if (
          typeof alias !== "string" ||
          !controller.store.get().models.some((m) => m.alias === alias)
        )
          throw new UiControlError("invalidParams", "alias must be a model the card offers");
        await controller.pick(alias);
        break;
      }
      case "later":
        controller.later();
        break;
      default:
        throw new UiControlError("invalidParams", "action must be read, check, pick or later");
    }
    return read(app);
  });
}
