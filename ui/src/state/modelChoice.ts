/**
 * The default model choice: the models Vibe's own config knows (`config/read`) and the
 * session-independent write that makes one the default (`setDefaultModel`).
 * Depends on: coreStore (`CoreController`), protocol/core types. Called by: ModelSettings.
 * Invariants:
 * - Models come from Vibe, never from a list in this app; an unreadable config is "no models yet"
 *   with the reason, not a guess.
 * - Settings writes only the new-session default. Per-chat picks still use `pickModel`;
 *   changing the default never changes an existing chat.
 * - Parsing is defensive: the result is untyped JSON from Vibe.
 */
import type { JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { newChatPick } from "../features/composer/model/models";
import { modelTerm } from "../vocabulary";

/** One model Vibe's config lists. */
export interface ModelOption {
  alias: string;
  label: string;
  /** Only levels actually reported for this model; absent means no explicit level is offered. */
  thinkingLevels?: readonly string[];
}

/** What the config says. */
export interface ModelList {
  models: readonly ModelOption[];
  /** Alias new chats start with (saved pin, else reported fallback), or null. */
  current: string | null;
}

function asObject(value: JsonValue | undefined): { [key in string]?: JsonValue } | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

/** Reads the models out of a `config/read` result; `null` when the shape is not recognised. */
export function parseModels(result: JsonValue): ModelList | null {
  const config = asObject(asObject(result)?.["config"]);
  const list = config?.["models"];
  if (config === null || !Array.isArray(list)) return null;
  const models: ModelOption[] = [];
  for (const entry of list) {
    const model = asObject(entry);
    const alias = model?.["alias"];
    if (typeof alias !== "string" || alias === "") continue;
    const display = model?.["displayName"];
    const levels = model?.["thinkingLevels"];
    models.push({
      alias,
      label: modelTerm(alias, display).name,
      ...(Array.isArray(levels)
        ? {
            thinkingLevels: [
              ...new Set(levels.filter((l): l is string => typeof l === "string" && l !== "")),
            ],
          }
        : {}),
    });
  }
  return { models, current: newChatPick(result).alias };
}

/** A session for settings RPCs that require one: the open one, else the most recently updated. */
export function sessionForPick(core: CoreController, selected: string | null): string | null {
  const sessions = core.store.get().sessions;
  if (selected !== null && sessions[selected] !== undefined) return selected;
  let best: { id: string; at: number } | null = null;
  for (const entry of Object.values(sessions)) {
    const at = entry.session["updatedAt"];
    const when = typeof at === "number" ? at : 0;
    if (best === null || when > best.at) best = { id: entry.id, at: when };
  }
  return best?.id ?? null;
}

/** Reads the model list; rejects with a plain-words `Error`. */
export async function readModels(core: CoreController): Promise<ModelList> {
  try {
    const result = await core.run({ type: "call", method: "config/read", params: {} });
    if (result.type !== "reply") throw new Error("unexpected answer");
    const parsed = parseModels(result.result);
    if (parsed === null) throw new Error("Vibe's config did not list models.");
    return parsed;
  } catch (thrown) {
    throw new Error(toCallError(thrown).error.message, { cause: thrown });
  }
}

/** Saves the new-session default without opening or changing a chat; rejects with a plain-words error. */
export async function writeDefaultModel(core: CoreController, alias: string): Promise<void> {
  try {
    await core.run({ type: "setDefaultModel", modelAlias: alias });
  } catch (thrown) {
    throw new Error(toCallError(thrown).error.message, { cause: thrown });
  }
}
