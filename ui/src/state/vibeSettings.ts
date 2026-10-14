/**
 * More of Vibe's own settings in Settings:
 * the default access mode for new sessions, background processes,
 * subagents, the stalled-answer timeout, and a model's nucleus sampling and output cap.
 * Read with Vibe's `config/fields/read` and written through a session with `config/write` (the
 * gate's allowlist checks every path and value).
 * Depends on: coreStore, protocol/core types, protocol/client (errors). Called by: settings/AgentDefaultsSettings.
 * Invariants:
 * - A value a higher layer sets (environment, administrator, this session's overrides) is shown as set there and not
 *   offered for editing (a write to the user's file would be shadowed).
 * - Full access as the default asks the native confirmation (the gate requires it), like switching to it.
 * - Subagents is never written through the open chat (Vibe refuses it there and breaks that chat): it is saved for
 * new chats through the core's short-lived chat (`writeSubagentsForNewChats`).
 * - Parsing is defensive: Vibe's answer is untyped JSON; what is not recognised is `null`.
 */
import type { JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { modelTerm } from "../vocabulary";

type Obj = { [key in string]?: JsonValue };

const SHADOWING = new Set(["environment", "admin", "overrides"]);

function asObject(value: JsonValue | undefined): Obj | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

/** One setting: its value and who else sets it. */
export interface Field<T> {
  value: T | null;
  /** The layer that decides it when that layer is not the user's file, else `null`. */
  setBy: string | null;
}

/** A configured model's tunables. */
export interface ModelTuning {
  alias: string;
  label: string;
  topP: number | null;
  maxOutputTokens: number | null;
}

/** What Settings shows. */
export interface VibeSettings {
  defaultAgent: Field<string>;
  activeModel: string | null;
  models: readonly ModelTuning[];
  backgroundProcesses: Field<boolean>;
  subagents: Field<boolean>;
  streamIdleTimeout: Field<number>;
}

function topLayer(field: Obj | null): string | null {
  const layers = field?.["layerValues"];
  if (!Array.isArray(layers) || layers.length === 0) return null;
  const layer = asObject(layers[0])?.["layer"];
  return typeof layer === "string" && SHADOWING.has(layer) ? layer : null;
}

function field<T>(
  byName: Map<string, Obj>,
  name: string,
  guard: (v: JsonValue | undefined) => v is T & JsonValue,
): Field<T> {
  const f = byName.get(name) ?? null;
  const v = f?.["value"];
  return { value: guard(v) ? v : null, setBy: topLayer(f) };
}

const isString = (v: JsonValue | undefined): v is string => typeof v === "string";
const isBool = (v: JsonValue | undefined): v is boolean => typeof v === "boolean";
const isNumber = (v: JsonValue | undefined): v is number => typeof v === "number";

function modelOf(value: JsonValue): ModelTuning | null {
  const m = asObject(value);
  const alias = m?.["alias"];
  if (m === null || typeof alias !== "string" || alias === "") return null;
  const display = m["display_name"] ?? m["displayName"];
  const topP = m["top_p"] ?? m["topP"];
  const cap = m["max_output_tokens"] ?? m["maxOutputTokens"];
  return {
    alias,
    label: modelTerm(alias, display).name,
    topP: typeof topP === "number" ? topP : null,
    maxOutputTokens: typeof cap === "number" ? cap : null,
  };
}

/** Reads the settings out of a `config/fields/read` result; `null` when the shape is not recognised. */
export function parseVibeSettings(result: JsonValue): VibeSettings | null {
  const fields = asObject(result)?.["fields"];
  if (!Array.isArray(fields)) return null;
  const byName = new Map<string, Obj>();
  for (const entry of fields) {
    const f = asObject(entry);
    const name = f?.["name"];
    if (f !== null && typeof name === "string") byName.set(name, f);
  }
  const models = byName.get("models")?.["value"];
  const modelList = Array.isArray(models)
    ? models
    : asObject(models) !== null
      ? Object.values(asObject(models) ?? {})
      : [];
  // The default model is the user's own value; a session's pick sits in the overrides layer on top (probe 2026-10-10).
  const activeField = byName.get("active_model");
  const layers = activeField?.["layerValues"];
  const own = Array.isArray(layers)
    ? layers.map((l) => asObject(l)).find((l) => l !== null && l["layer"] !== "overrides")?.[
        "value"
      ]
    : undefined;
  const active = own ?? activeField?.["value"];
  return {
    defaultAgent: field(byName, "default_agent", isString),
    activeModel: typeof active === "string" ? active : null,
    models: modelList.flatMap((m) => {
      const t = m === undefined ? null : modelOf(m);
      return t === null ? [] : [t];
    }),
    backgroundProcesses: field(byName, "enable_background_processes", isBool),
    subagents: field(byName, "enable_subagents", isBool),
    streamIdleTimeout: field(byName, "api_stream_idle_timeout", isNumber),
  };
}

function message(thrown: unknown): Error {
  return new Error(toCallError(thrown).error.message, { cause: thrown });
}

/** Reads the settings through `sessionId`. */
export async function readVibeSettings(
  core: CoreController,
  sessionId: string,
): Promise<VibeSettings> {
  try {
    const result = await core.run({
      type: "call",
      method: "config/fields/read",
      params: { sessionId },
    });
    if (result.type !== "reply") throw new Error("unexpected answer");
    const parsed = parseVibeSettings(result.result);
    if (parsed === null) throw new Error("Vibe's config did not list its settings.");
    return parsed;
  } catch (thrown) {
    throw message(thrown);
  }
}

/** One `config/write` op. */
export type ConfigOp =
  { op: "set"; path: string; value: JsonValue } | { op: "remove"; path: string };

/**
 * Writes `ops` with Vibe's `config/write` through `sessionId` (reloading the session's runtime). `confirm`: the write
 * is an escalation (Full access as the default), so the native confirmation is asked first.
 */
export async function writeConfig(
  core: CoreController,
  sessionId: string,
  ops: readonly ConfigOp[],
  reason: string,
  confirm = false,
): Promise<void> {
  const params = {
    sessionId,
    ops: ops as unknown as JsonValue,
    reason,
    reloadRuntime: true,
  } satisfies Record<string, JsonValue>;
  try {
    // Only an escalation (Full access as the default) is confirmed first; asking for others would be refused.
    const answer = confirm
      ? await core.run({ type: "requestConfirmation", method: "config/write", params })
      : null;
    const result = await core.run({
      type: "call",
      method: "config/write",
      params,
      ...(answer?.type === "confirmation" ? { confirmationToken: answer.token } : {}),
    });
    if (result.type === "reply") {
      const body = asObject(result.result);
      if (body?.["rejected"] === true) throw new Error("Vibe did not accept the change.");
      const failures = body?.["failures"];
      if (Array.isArray(failures) && failures.length > 0)
        throw new Error("Vibe could not save the change.");
    }
  } catch (thrown) {
    throw message(thrown);
  }
}

/** What saving Subagents for new chats came back with. */
export interface SubagentsSaved {
  /** The value a new chat starts with, as Vibe reported it; `null` when it could not be read back. */
  enabled: boolean | null;
  /** The short-lived chat the core used and could not delete (its id), else `null`. */
  leftover: string | null;
}

/**
 * Saves the Subagents setting for new chats. Vibe 2.26.1 refuses to change it through a chat that already exists
 * (and then that chat fails every later turn), so this never goes through the open chat: the core writes it through
 * a short-lived empty chat of its own and deletes that chat again (`sessionTools` op `setSubagents`). Chats that are
 * already open keep the value they started with.
 */
export async function writeSubagentsForNewChats(
  core: CoreController,
  enabled: boolean,
): Promise<SubagentsSaved> {
  try {
    const answer = await core.run({
      type: "sessionTools",
      request: { op: "setSubagents", enabled },
    });
    if (answer.type !== "sessionTools" || answer.result.kind !== "subagentsSet")
      throw new Error("unexpected answer");
    return { enabled: answer.result.enabled, leftover: answer.result.leftoverSessionId };
  } catch (thrown) {
    throw message(thrown);
  }
}

/** The JSON-pointer path of a model field (`/models/<alias>/<field>`; `~` and `/` escaped). */
export function modelPath(alias: string, fieldName: "top_p" | "max_output_tokens"): string {
  return `/models/${alias.replace(/~/g, "~0").replace(/\//g, "~1")}/${fieldName}`;
}
