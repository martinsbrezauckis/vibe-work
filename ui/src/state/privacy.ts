/**
 * The two privacy switches: Vibe's usage telemetry (`enable_telemetry`) and its
 * experiments (`experiments.enable`). Mistral's remote experiments decide which features an account gets, and they only
 * run with telemetry on, so telemetry off also hides rollout features such as smart
 * approve and extra models. The screen says that in plain words; this module computes the facts it
 * shows.
 * Depends on: coreStore, protocol/core types.
 * Called by:
 * features/settings/PrivacySettings, settingsRoutes.
 * Invariants:
 * - The guarded runtime disables usage telemetry and remote experiments. This screen reports both off
 *   and managed by Vibe Work even if a saved configuration contains an opt-in.
 * - Raw field parsing retains the upstream layer names. The managed policy takes precedence in this screen.
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

/** Reports the guarded runtime's policy without depending on a chat or upstream config fields. */
export function readPrivacy(): Promise<PrivacyView> {
  return Promise.resolve({
    telemetry: { on: false, setBy: "environment" },
    experiments: { on: false, setBy: "environment" },
  });
}

/** Which switch. */
export type SwitchName = "telemetry" | "experiments";

const PATHS: Record<SwitchName, string> = {
  telemetry: "/enable_telemetry",
  experiments: "/experiments/enable",
};

/**
 * Saves an explicit opt-out; enabling is refused before sending a core command.
 * `reloadRuntime` makes Vibe re-derive the session's settings.
 */
export async function writePrivacy(
  core: CoreController,
  sessionId: string,
  which: SwitchName,
  on: boolean,
): Promise<void> {
  if (on) throw new Error("Usage data and experiments are disabled by Vibe Work.");
  const params = {
    sessionId,
    ops: [{ op: "set", path: PATHS[which], value: on }],
    reason: "privacy switch off",
    reloadRuntime: true,
  } satisfies Record<string, JsonValue>;
  const send = () =>
    core.run({
      type: "call",
      method: "config/write",
      params,
    });
  try {
    const result = await send();
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
