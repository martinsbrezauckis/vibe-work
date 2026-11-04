/** Browser settings actions shared with the Debug API. Uses the same commands as the visible controls. */
import { TEXT } from "../../../vocabulary";
import type { ChromeControlRequest } from "../../../protocol/core";
import { toCallError } from "../../../protocol/client/types";
import { UiControlError } from "../../../state/uiControl";
import type { App } from "../../shell/AppContext";
import { chromeAvailable, setupLine } from "./chromeRows";

/** Runs the action; returns the view after it (and the offer, for `offer`). */
export async function chromeAction(
  app: App,
  input: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const core = app.core;
  if (core === null) throw new UiControlError("notFound", "this run has no core");
  if (!chromeAvailable(core.store.get().chromeControl))
    throw new UiControlError("notFound", TEXT.chromeUnavailable);
  const action = input["do"] ?? "read";
  let request: ChromeControlRequest;
  switch (action) {
    case "read":
      request = { op: "read" };
      break;
    case "enabled":
      if (typeof input["enabled"] !== "boolean")
        throw new UiControlError("invalidParams", "enabled must be a boolean");
      request = { op: "setEnabled", enabled: input["enabled"] };
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
    case "connect":
      request = { op: "connect" };
      break;
    case "store":
      request = { op: "openStore" };
      break;
    case "remove":
      request = { op: "remove" };
      break;
    default:
      throw new UiControlError(
        "invalidParams",
        "do must be read, enabled, offer, setup, connect, store or remove",
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
