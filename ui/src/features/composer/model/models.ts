/**
 * The model and thinking pickers' data: the configured models from a runtime
 * snapshot (`RuntimeSnapshot.config.models`, `ModelConfigView`) or from `config/read` (`ConfigView`),
 * the session's current pick, and the thinking levels a model allows.
 * Depends on: protocol/core types, contextWindow (published full-window labels). Called by: the model menu,
 * the composer view, composerSession. Context labels never use stats or the runtime's compaction threshold.
 * Invariants: pure; unknown shapes give empty lists, never a throw. No cost hint: `ModelConfigView`
 * carries no prices (Vibe keeps prices in the `[[models]]` TOML entries, which the runtime view drops), so
 * the picker shows none rather than a guess.
 */
import type { JsonValue } from "../../../protocol/core";
import { offeredReasoning } from "../../../state/defaultReasoning";
import { modelTerm, TEXT, vocabulary } from "../../../vocabulary";
import { type Obj, obj } from "../../transcript/model/entries";
import { contextWindowLabel } from "./contextWindow";

/** Vibe's thinking levels (`ThinkingLevel`), lowest first. */
export const THINKING_LEVELS = Object.keys(vocabulary.thinkingLevels);

/** One configured model. */
export interface ModelChoice {
  alias: string;
  label: string;
  /** Levels this model accepts (all when Vibe does not say). */
  thinkingLevels: readonly string[];
  supportsImages: boolean;
  /** One full-window label, from Mistral's model card or the custom model's configured count. */
  contextLabel: string;
  /** API model identity for the provider's thinking mapping, independent of a user alias. */
  apiModel?: string;
  /** Provider identity, so Mistral wire explanations do not describe a different backend. */
  provider?: string;
}

/** The `ConfigView` inside a runtime snapshot (`{config}`), a `config/read` reply, or itself. */
function configView(source: JsonValue | undefined): Obj | null {
  const o = obj(source);
  if (o === null) return null;
  const inner = obj(o["config"]);
  return inner ?? (Array.isArray(o["models"]) ? o : null);
}

function choiceOf(value: JsonValue): ModelChoice | null {
  const m = obj(value);
  const alias = m?.["alias"];
  if (m === null || typeof alias !== "string" || alias === "") return null;
  const display = m["displayName"];
  const levels = m["thinkingLevels"];
  return {
    alias,
    label: modelTerm(alias, display).name,
    thinkingLevels: Array.isArray(levels)
      ? levels.filter((l): l is string => typeof l === "string")
      : THINKING_LEVELS,
    supportsImages: m["supportsImages"] === true,
    contextLabel: contextWindowLabel(m),
    ...(typeof m["provider"] === "string" ? { provider: m["provider"] } : {}),
    ...(typeof m["name"] === "string" ? { apiModel: m["name"] } : {}),
  };
}

/** The configured models, in Vibe's order. */
export function modelChoices(source: JsonValue | undefined): ModelChoice[] {
  const config = configView(source);
  const models = config?.["models"];
  if (!Array.isArray(models)) return [];
  const active = obj(config?.["activeModel"]);
  return models.flatMap((m) => {
    const row = obj(m);
    const choice = choiceOf(
      active !== null && row?.["alias"] === active["alias"] ? { ...row, ...active } : m,
    );
    return choice === null ? [] : [choice];
  });
}

/** The session's model alias and thinking level, as far as known. */
export interface CurrentPick {
  alias: string | null;
  effort: string | null;
  modelName?: string;
  modelDisplayName?: string;
}

/** Carry only the selected row's identity across startup; availability always comes from the core. */
export function selectedModel(
  source: JsonValue | undefined,
  alias: string | undefined | null,
): {
  modelName?: string;
  modelDisplayName?: string;
} {
  const model = modelChoices(source).find((model) => model.alias === alias);
  return model?.apiModel === undefined
    ? {}
    : { modelName: model.apiModel, modelDisplayName: model.label };
}

/**
 * The current pick: the runtime's active model (`config.activeModel.alias`, its `thinking`) when known,
 * else the session's own `model` / `reasoningEffort` fields.
 */
export function currentPick(runtime: JsonValue | undefined, session: Obj | undefined): CurrentPick {
  const active = obj(configView(runtime)?.["activeModel"] ?? null);
  const alias = active?.["alias"];
  const thinking = active?.["thinking"];
  const model = session?.["model"];
  const effort = session?.["reasoningEffort"];
  return {
    alias: typeof alias === "string" ? alias : typeof model === "string" ? model : null,
    effort: typeof thinking === "string" ? thinking : typeof effort === "string" ? effort : null,
  };
}

/**
 * Resolves a new chat's held pick, then its configured pin, then the reported fallback.
 * A pinned active model takes precedence over the experiment fallback. Thinking follows the
 * selected model's reported default, with an offered saved preference or explicit held level taking precedence.
 */
export function newChatPick(
  config: JsonValue | undefined,
  held: CurrentPick = { alias: null, effort: null },
  defaultReasoning: string | null = null,
): CurrentPick {
  const view = configView(config);
  const active = currentPick(config, undefined);
  const fallback = view?.["defaultModelAlias"];
  const configuredAlias =
    view?.["activeModelPinned"] === true && active.alias !== null
      ? active.alias
      : typeof fallback === "string" && fallback !== ""
        ? fallback
        : active.alias;
  const alias = held.alias ?? configuredAlias;
  const models = view?.["models"];
  const model = Array.isArray(models) ? models.find((m) => obj(m)?.["alias"] === alias) : undefined;
  const thinking = obj(model)?.["thinking"];
  const effort =
    alias === active.alias && active.effort !== null
      ? active.effort
      : typeof thinking === "string"
        ? thinking
        : null;
  return {
    alias,
    effort: held.effort ?? configuredReasoning(config, alias, defaultReasoning) ?? effort,
  };
}

/** A saved new-chat preference admitted by the resolved model's reported levels, else no override. */
export function configuredReasoning(
  config: JsonValue | undefined,
  alias: string | null,
  level: string | null,
): string | null {
  const models = configView(config)?.["models"];
  const model = Array.isArray(models) ? models.find((m) => obj(m)?.["alias"] === alias) : undefined;
  return offeredReasoning(level, obj(model)?.["thinkingLevels"]);
}

/** The saved model pin to apply before a first turn; unpinned defaults stay unpinned. */
export function configuredModelPin(config: JsonValue | undefined): string | null {
  return configView(config)?.["activeModelPinned"] === true
    ? currentPick(config, undefined).alias
    : null;
}

/** The label for an alias (the alias itself when the model is not configured). */
export function modelLabel(choices: readonly ModelChoice[], alias: string | null): string {
  if (alias === null) return TEXT.model;
  return choices.find((c) => c.alias === alias)?.label ?? alias;
}

/** Explains when Off and Low are the same on the selected model. */
export function thinkingRequest(model: ModelChoice | undefined, level: string): string {
  if (!model?.apiModel) return "";
  if (model.provider !== undefined && model.provider !== "mistral") {
    return "";
  }
  const knownNone = [
    "mistral-large-4",
    "mistral-large-4-0",
    "mistral-small-latest",
    "mistral-small-2603",
    "mistral-medium-latest",
    "mistral-medium-3-5",
  ].includes(model.apiModel);
  if (level === "off" || level === "low") {
    return knownNone && model.thinkingLevels.includes("low")
      ? "Off and Low are the same on this model."
      : "";
  }
  return "";
}
