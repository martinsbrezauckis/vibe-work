/**
 * The `chrome` action family of the `ui/settings` UI-control route (Settings › Browser › Your Chrome). Each action
 * sends the same `chromeControl` request the section's control sends (with the Debug API origin):
 * - `{do: "read"}` (default): the view as the section shows it (status line, chosen level or Custom, every row);
 * - `level {level}` (a level card);
 * - `offer` (Set up…: the offer the dialog shows), `setup {offerId}` (its Set up button), `store` (Open the store
 *   page again), `remove` (Remove's confirm button).
 * A typed level request receives a one-use core grant for that exact level; generic clicks get no grant.
 * Depends on: state/uiControl, AppContext types, chromeRows. Called by: settingsRoutes.
 */
import type { ChromeControlRequest, ChromeLevel } from "../../../protocol/core";
import { toCallError } from "../../../protocol/client/types";
import { UiControlError } from "../../../state/uiControl";
import type { App } from "../../shell/AppContext";
import { LEVELS, chromeAvailable, setupLine } from "./chromeRows";

function pick<T extends string>(
  input: Record<string, unknown>,
  name: string,
  values: readonly T[],
): T {
  const value = input[name];
  if (typeof value === "string" && (values as readonly string[]).includes(value)) return value as T;
  throw new UiControlError("invalidParams", `${name} must be one of ${values.join(", ")}`);
}

/** Runs the action; returns the view after it (and the offer, for `offer`). */
export async function chromeAction(
  app: App,
  input: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const core = app.core;
  if (core === null) throw new UiControlError("notFound", "this run has no core");
  if (!chromeAvailable(core.store.get().chromeControl))
    throw new UiControlError("notFound", "Chrome Control is not in this version of Vibe Work.");
  const action = input["do"] ?? "read";
  let request: ChromeControlRequest;
  switch (action) {
    case "read":
      request = { op: "read" };
      break;
    case "level":
      request = { op: "setLevel", level: pick<ChromeLevel>(input, "level", LEVELS) };
      break;
    case "offer":
      request = { op: "offerSetup" };
      break;
    case "setup": {
      const offerId = input["offerId"];
      if (typeof offerId !== "string" || offerId === "")
        throw new UiControlError("invalidParams", "offerId must be a non-empty string");
      request = { op: "setup", offerId };
      break;
    }
    case "store":
      request = { op: "openStore" };
      break;
    case "remove":
      request = { op: "remove" };
      break;
    default:
      throw new UiControlError(
        "invalidParams",
        "do must be read, level, offer, setup, store or remove",
      );
  }
  try {
    const result = await core.run({ type: "chromeControl", request });
    if (result.type !== "chromeControl") throw new Error("Unexpected chromeControl response.");
    return {
      view: result.view,
      status: setupLine(result.view),
      offer: result.offer,
    };
  } catch (thrown) {
    if (thrown instanceof UiControlError) throw thrown;
    const error = toCallError(thrown).error;
    throw new UiControlError(error.code, error.message);
  }
}
