/**
 * The two privacy switches: Vibe's usage telemetry (`enable_telemetry`) and its
 * experiments (`experiments.enable`), read with `config/fields/read` and written with `config/write`
 * through a session. Mistral's remote experiments decide which features an account gets, and they only
 * run with telemetry on, so telemetry off also hides rollout features such as smart
 * approve and extra models. The screen says that in plain words; this module computes the facts it
 * shows.
 * Depends on: coreStore, modelChoice (`sessionForPick`), protocol/core types. Called by:
 * features/settings/PrivacySettings, settingsRoutes.
 * Invariants:
 * - Turning a switch OFF is sent as is. Turning it ON (data leaves the machine again) needs the native
 *   confirmation: this module asks for it first and sends the call with the token.
 * - A value that a higher layer sets (`environment`, `admin`, `overrides`) is shown as set by someone
 *   else and is not editable here: a write to the user layer would be shadowed.
 * - Parsing is defensive: the result is untyped JSON from Vibe; what is not recognised is "unknown".
 */
import type { JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";

/** Layers that shadow what this screen writes. */
const SHADOWING = new Set(["environment", "admin", "overrides"]);

/** One switch as Vibe reports it. */
export interface SwitchView {
  /** The effective value, or `null` when Vibe did not report it. */
  on: boolean | null;
  /** The layer that decides it when that layer is not the user's file, else `null`. */
  setBy: string | null;
}

/** Both switches. */
export interface PrivacyView {
  telemetry: SwitchView;
  experiments: SwitchView;
}

type Obj = { [key in string]?: JsonValue };

function asObject(value: JsonValue | undefined): Obj | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

function topLayer(field: Obj | null): string | null {
  const layers = field?.["layerValues"];
  if (!Array.isArray(layers) || layers.length === 0) return null;
  const first = asObject(layers[0]);
  const layer = first?.["layer"];
  return typeof layer === "string" && SHADOWING.has(layer) ? layer : null;
}

/** Reads the two switches out of a `config/fields/read` result; `null` when the shape is not recognised. */
export function parsePrivacy(result: JsonValue): PrivacyView | null {
  const fields = asObject(result)?.["fields"];
  if (!Array.isArray(fields)) return null;
  const byName = new Map<string, Obj>();
  for (const entry of fields) {
    const field = asObject(entry);
    const name = field?.["name"];
    if (field !== null && typeof name === "string") byName.set(name, field);
  }
  const telemetry = byName.get("enable_telemetry") ?? null;
  const experiments = byName.get("experiments") ?? null;
  if (telemetry === null && experiments === null) return null;
  const telemetryValue = telemetry?.["value"];
  const experimentsValue = asObject(experiments?.["value"])?.["enable"];
  return {
    telemetry: {
      on: typeof telemetryValue === "boolean" ? telemetryValue : null,
      setBy: topLayer(telemetry),
    },
    experiments: {
      on: typeof experimentsValue === "boolean" ? experimentsValue : null,
      setBy: topLayer(experiments),
    },
  };
}

/** Reads the switches through `sessionId`; rejects with a plain-words `Error`. */
export async function readPrivacy(core: CoreController, sessionId: string): Promise<PrivacyView> {
  try {
    const result = await core.run({
      type: "call",
      method: "config/fields/read",
      params: { sessionId },
    });
    if (result.type !== "reply") throw new Error("unexpected answer");
    const parsed = parsePrivacy(result.result);
    if (parsed === null) throw new Error("Vibe's config did not list the privacy switches.");
    return parsed;
  } catch (thrown) {
    throw new Error(toCallError(thrown).error.message, { cause: thrown });
  }
}

/** Which switch. */
export type SwitchName = "telemetry" | "experiments";

const PATHS: Record<SwitchName, string> = {
  telemetry: "/enable_telemetry",
  experiments: "/experiments/enable",
};

/**
 * Sets a switch; rejects with a plain-words `Error`. `reloadRuntime` makes Vibe re-derive the session's
 * settings now, so the access menu and model list follow (smart approve appears only with telemetry on).
 */
export async function writePrivacy(
  core: CoreController,
  sessionId: string,
  which: SwitchName,
  on: boolean,
): Promise<void> {
  const params = {
    sessionId,
    ops: [{ op: "set", path: PATHS[which], value: on }],
    reason: on ? "privacy switch on" : "privacy switch off",
    reloadRuntime: true,
  } satisfies Record<string, JsonValue>;
  const send = (token?: string) =>
    core.run({
      type: "call",
      method: "config/write",
      params,
      ...(token === undefined ? {} : { confirmationToken: token }),
    });
  try {
    let result;
    if (on) {
      // Turning data sharing on is an escalation the gate answers with `confirmationRequired`; asking for
      // the confirmation first avoids a refused call (and its "refused" notice) on the way.
      const answer = await core
        .run({ type: "requestConfirmation", method: "config/write", params })
        .catch((thrown: unknown) => {
          if (toCallError(thrown).error.code === "confirmationNotApplicable") return null;
          throw thrown;
        });
      result = await send(answer?.type === "confirmation" ? answer.token : undefined);
    } else {
      result = await send();
    }
    if (result.type === "reply") {
      const body = asObject(result.result);
      if (body?.["rejected"] === true) throw new Error("Vibe did not accept the change.");
      const failures = body?.["failures"];
      if (Array.isArray(failures) && failures.length > 0)
        throw new Error("Vibe could not save the change.");
    }
  } catch (thrown) {
    throw new Error(toCallError(thrown).error.message, { cause: thrown });
  }
}
