/**
 * The compaction point of each configured model, for Settings › Models and agent: when Vibe compacts a chat's
 * history, as Vibe 3.0.3's per-model `auto_compact_threshold`. Read from a `config/fields/read` result (the `models`
 * field's per-layer values), written by Settings as `/models/<alias>/auto_compact_threshold` through `config/write`.
 * Depends on: protocol/core types. Called by: state/vibeSettings (parse), settings/CompactionRows (render).
 * Invariants:
 * - The choices lie inside the range Vibe allows for the model's window (Vibe's own rule, `bounds.py`), so a pick is
 *   never refused for its size; Vibe still checks every write.
 * - The window is the one Vibe uses without the network: the configured `max_context_length`, else Vibe's shipped
 *   table of known models, else Vibe's assumed 262,144. A model Vibe does not know gets that assumed window's range.
 * - "Default" removes the user's own value; what it then means (another layer's value, else Vibe's default for the
 *   window) is shown in its label. A value a higher layer sets is shown with that layer and is not editable.
 * - Parsing is defensive: what is not recognised gives no row, never a throw.
 */
import type { JsonValue } from "../protocol/core";

type Obj = { [key in string]?: JsonValue };

/** Vibe's assumed window for a model it does not know (`bounds.py` `DEFAULT_CONTEXT_WINDOW`). */
export const ASSUMED_WINDOW = 262_144;

/**
 * Windows Vibe ships for the Mistral models it knows, by API name (Vibe 3.0.3
 * `completion_profile/known_models.py`; refresh on each Vibe update).
 */
const KNOWN_WINDOWS: ReadonlyMap<string, number> = new Map([
  ...[
    "mistral-medium-3.5",
    "mistral-medium-3-5",
    "mistral-medium-2604",
    "mistral-medium-latest",
    "magistral-medium-latest",
    "mistral-vibe-cli-latest",
    "mistral-vibe-cli-with-tools",
    "mistral-large-2512",
    "mistral-large-latest",
  ].map((name): [string, number] => [name, 262_144]),
  ...["mistral-large-4", "mistral-large-4-0", "zai-glm-5-3", "zai-glm-5", "zai-glm-latest"].map(
    (name): [string, number] => [name, 1_048_576],
  ),
]);

/** The points offered, where the window's range allows them. */
const LADDER = [
  50_000, 100_000, 150_000, 200_000, 256_000, 300_000, 400_000, 500_000, 600_000, 700_000, 800_000,
  900_000,
];

/** Layers above the user's file: a threshold set there wins over the user's. */
const ABOVE_USER = new Set(["project-toml", "environment", "overrides", "agent-profile", "admin"]);
const USER = new Set(["user-toml", "user-supplied"]);

/** The range a threshold must fall in for a window, and Vibe's default inside it (`bounds.py`). */
export interface ThresholdBounds {
  minimum: number;
  maximum: number;
  default: number;
}

/** Vibe's `threshold_bounds`, or `null` for a window too small to compact in. */
export function thresholdBounds(window: number): ThresholdBounds | null {
  const minimum = Math.max(Math.min(50_000, Math.floor(window / 2)), 10_000);
  const maximum = window - Math.min(24_000, Math.ceil(window / 4));
  if (maximum < minimum) return null;
  const preferred = Math.min(256_000, window - 60_000);
  return { minimum, maximum, default: Math.max(minimum, Math.min(preferred, maximum)) };
}

/** One model's compaction point. */
export interface CompactionRow {
  alias: string;
  /** The model's raw identity, for its context label. */
  model: Obj;
  /** The value in force when it is not Default: the user's own, or the higher layer's that decides it. */
  own: number | null;
  /** What Default means for this model. */
  fallback: number;
  /** The points offered (the user's own value included), lowest first. */
  choices: readonly number[];
  /** The layer that decides it when that layer is above the user's file, else `null`. */
  setBy: string | null;
}

function asObject(value: JsonValue | undefined): Obj | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

const count = (v: JsonValue | undefined): number | null =>
  typeof v === "number" && Number.isSafeInteger(v) && v > 0 ? v : null;

/** One layer's models by alias (Vibe gives a map; a list of entries is read too). */
function layerModels(value: JsonValue | undefined): Map<string, Obj> {
  const out = new Map<string, Obj>();
  const entries = Array.isArray(value) ? value : Object.values(asObject(value) ?? {});
  for (const entry of entries) {
    const m = asObject(entry);
    const alias = m?.["alias"];
    if (m !== null && typeof alias === "string" && alias !== "") out.set(alias, m);
  }
  return out;
}

/** The window Vibe uses for a merged model entry when it cannot look the model up. */
function windowOf(model: Obj, name: string): number {
  return (
    count(model["max_context_length"] ?? model["maxContextLength"]) ??
    KNOWN_WINDOWS.get(name.toLowerCase().split("/").pop() ?? name) ??
    ASSUMED_WINDOW
  );
}

/** The global threshold (Vibe's `auto_compact_threshold` for models without their own) and whether an
 * administrator forces it onto every model. Only read when Vibe lists the field at its global path. */
function globalThreshold(field: Obj | undefined): { value: number | null; admin: boolean } {
  const top = asObject(Array.isArray(field?.["layerValues"]) ? field["layerValues"][0] : undefined);
  if (field?.["path"] !== "/auto_compact_threshold" || top === null)
    return { value: null, admin: false };
  return {
    value: top["layer"] === "default" ? null : count(top["value"]),
    admin: top["layer"] === "admin",
  };
}

/** The compaction rows of a `config/fields/read` result's fields, in the order Vibe lists the models. */
export function compactionRows(byName: ReadonlyMap<string, Obj>): CompactionRow[] {
  const field = byName.get("models");
  const layers = Array.isArray(field?.["layerValues"]) ? field["layerValues"] : [];
  const named = layers.flatMap((l) => {
    const o = asObject(l);
    const layer = o?.["layer"];
    return o === null || typeof layer !== "string"
      ? []
      : [{ layer, models: layerModels(o["value"]) }];
  });
  const merged = layerModels(field?.["value"]);
  // An administrator's global threshold is forced onto every model (Vibe's admin layer).
  const global = globalThreshold(byName.get("auto_compact_threshold"));
  return [...merged.entries()].flatMap(([alias, model]) => {
    const name = typeof model["name"] === "string" ? model["name"] : alias;
    const bounds = thresholdBounds(windowOf(model, name));
    if (bounds === null) return [];
    const at = (layer: { models: Map<string, Obj> }) =>
      count(layer.models.get(alias)?.["auto_compact_threshold"]);
    const above = named.find((n) => ABOVE_USER.has(n.layer) && at(n) !== null);
    const user = named.find((n) => USER.has(n.layer));
    const own = user === undefined ? null : at(user);
    const userIndex = user === undefined ? -1 : named.indexOf(user);
    const below = named.slice(userIndex + 1).find((n) => !USER.has(n.layer) && at(n) !== null);
    // Vibe keeps a model's own threshold from any layer, else takes the global one, else its window's default.
    const fallback = (below === undefined ? null : at(below)) ?? global.value ?? bounds.default;
    const shown = global.admin ? global.value : above === undefined ? own : at(above);
    const inRange = LADDER.filter((n) => n >= bounds.minimum && n <= bounds.maximum);
    const choices = [...new Set([...inRange, ...(shown === null ? [] : [shown])])].sort(
      (a, b) => a - b,
    );
    return [
      {
        alias,
        model,
        own: shown,
        fallback,
        choices,
        setBy: global.admin ? "admin" : (above?.layer ?? null),
      },
    ];
  });
}
