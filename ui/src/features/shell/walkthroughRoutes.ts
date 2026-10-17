/** Walkthrough Debug API handler. Validates caption updates; called by useShellWiring, never by normal controls. */
import { UiControlError } from "../../state/uiControl";
import type { App } from "./AppContext";

/** Registers caption state and lifecycle operations, with read-only polling during long UI actions. */
export function registerWalkthroughRoutes(app: App): () => void {
  const walkthrough = app.uiControl.walkthrough;
  const remove = app.uiControl.register(
    "walkthrough",
    (body) => {
      if (body !== undefined && body !== null && (typeof body !== "object" || Array.isArray(body)))
        throw new UiControlError("invalidParams", "the body must be a JSON object");
      const input = (body ?? {}) as Record<string, unknown>;
      const { action = "read", step, total, label } = input;
      const invalid = () => {
        throw new UiControlError(
          "invalidParams",
          "use read, start {total}, step {step, label}, pause, finish or stop",
        );
      };
      switch (action) {
        case "read":
          break;
        case "start":
          if (typeof total !== "number" || !Number.isSafeInteger(total) || total < 1)
            return invalid();
          walkthrough.start(total);
          break;
        case "step":
          if (
            typeof step !== "number" ||
            !Number.isSafeInteger(step) ||
            step < 1 ||
            step > walkthrough.store.get().total ||
            typeof label !== "string"
          )
            return invalid();
          walkthrough.step(step, label);
          break;
        case "pause":
          walkthrough.pause();
          break;
        case "finish":
          walkthrough.finish();
          break;
        case "stop":
          walkthrough.stop();
          break;
        default:
          return invalid();
      }
      return walkthrough.store.get();
    },
    (body) =>
      body === undefined ||
      body === null ||
      (typeof body === "object" && (!("action" in body) || body.action === "read")),
  );
  return () => {
    remove();
    walkthrough.stop();
  };
}
