/**
 * Initializes composer chips from the session's effective runtime before interaction or a first turn.
 * Depends on core/composer stores and Vibe's config provenance; called by composerSession.
 * Reads are coalesced across origins. Project thinking wins over the app preference; explicit picks
 * and existing conversations are never initialized again. A failed read/write leaves a turn unsent.
 */
import type { JsonValue } from "../protocol/core";
import {
  configuredModelPin,
  configuredReasoning,
  currentPick,
  type CurrentPick,
} from "../features/composer/model/models";
import { obj } from "../features/transcript/model/entries";
import type { ComposerStore } from "./composerStore";
import type { CoreController } from "./coreStore";

/** Shared by all of the composer's origin-bound action sets. */
export interface ComposerInitialization {
  ready: Set<string>;
  pending: Map<string, Promise<boolean>>;
  choices: Map<string, CurrentPick>;
}

/** A resolved transport call can still report a rejected configuration mutation. */
export function modelWriteAccepted(result: JsonValue | null): boolean {
  const response = obj(result);
  const failures = response?.["failures"];
  return response?.["rejected"] !== true && (!Array.isArray(failures) || failures.length === 0);
}

/** Whether the project explicitly sets thinking for this alias (including a value equal to its default). */
export function projectThinking(fields: JsonValue, alias: string | null): boolean {
  const rows = obj(fields)?.["fields"];
  if (!Array.isArray(rows) || alias === null) return false;
  for (const row of rows) {
    const field = obj(row);
    const layers = field?.["layerValues"];
    if (!Array.isArray(layers)) continue;
    for (const layer of layers) {
      const entry = obj(layer);
      if (entry?.["layer"] !== "project-toml") continue;
      if (field?.["name"] === "models") {
        const models = obj(entry["value"]);
        if (typeof obj(models?.[alias])?.["thinking"] === "string") return true;
      }
    }
  }
  return false;
}

/** Reads and admits defaults once; fresh lobby starts can identify themselves before being listed. */
export function prepareComposerSession(
  core: CoreController,
  composer: ComposerStore,
  initialization: ComposerInitialization,
  sessionId: string,
  fresh = false,
  given?: CurrentPick,
): Promise<boolean> {
  const held = given ?? {
    alias: null,
    effort: null,
  };
  const explicit = held.alias !== null || held.effort !== null;
  const previous = initialization.choices.get(sessionId);
  const same = previous?.alias === held.alias && previous.effort === held.effort;
  const pending = initialization.pending.get(sessionId);
  if (pending !== undefined)
    return explicit && !same
      ? pending.then((ok) =>
          ok
            ? prepareComposerSession(core, composer, initialization, sessionId, false, held)
            : false,
        )
      : pending;
  if (initialization.ready.has(sessionId) && (!explicit || same)) return Promise.resolve(true);
  initialization.choices.set(sessionId, held);
  const read = async (method: string): Promise<JsonValue | null> => {
    const reply = await core.run({ type: "call", method, params: { sessionId } });
    return reply.type === "reply" ? reply.result : null;
  };
  const prepare = async (): Promise<boolean> => {
    try {
      const entry = core.store.get().sessions[sessionId];
      let runtime = entry?.runtime ?? obj(await read("runtime/read"))?.["runtime"];
      if (runtime === undefined || obj(runtime) === null) return false;
      const pick = currentPick(runtime, entry?.session);
      const history = entry?.state?.["history"];
      const isFresh =
        (fresh || entry?.localOnly === true) && (!Array.isArray(history) || history.length === 0);
      const preference = isFresh ? composer.store.get().defaultReasoning : null;
      const alias = held.alias ?? pick.alias;
      let effort = held.effort;
      if (effort === null && preference !== null) {
        const fields = await read("config/fields/read");
        if (fields === null) return false;
        effort = projectThinking(fields, alias)
          ? null
          : configuredReasoning(runtime, alias, preference);
      }
      if (
        held.alias !== null ||
        held.effort !== null ||
        (effort !== null && effort !== pick.effort)
      ) {
        const pin = held.alias ?? configuredModelPin(runtime);
        const reply = await core.run({
          type: "pickModel",
          sessionId,
          ...(pin === null ? {} : { modelAlias: pin }),
          ...(effort === null ? {} : { reasoningEffort: effort }),
        });
        if (reply.type !== "reply" || !modelWriteAccepted(reply.result)) return false;
        runtime = obj(reply.result)?.["runtime"] ?? obj(await read("runtime/read"))?.["runtime"];
        const actual = currentPick(runtime, undefined);
        if (
          runtime === undefined ||
          (held.alias !== null && actual.alias !== held.alias) ||
          (effort !== null && actual.effort !== effort)
        )
          return false;
      }
      core.setRuntime(sessionId, runtime);
      initialization.ready.add(sessionId);
      return true;
    } catch {
      return false;
    }
  };
  const promise = prepare().finally(() => initialization.pending.delete(sessionId));
  initialization.pending.set(sessionId, promise);
  return promise;
}
