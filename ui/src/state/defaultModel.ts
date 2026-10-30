/**
 * The launch check of the Settings default model. Once Vibe is running the window asks the core once
 * (`checkDefaultModel`); when the core names a default Vibe no longer offers, the window asks which model to use from
 * now on (DefaultModelDialog) with the models Vibe offers (`config/read`), instead of failing or switching silently.
 * Depends on: coreStore (`run`, the server phase), modelChoice (model list, the default write), vocabulary. Called by: createApp (controller), useShellWiring (`watch`), DefaultModelDialog, defaultModelRoutes.
 * Invariants: one check per time the server reaches running; a failed check or read shows nothing (Settings still
 * offers the choice); only the person's pick (or the Debug API's) writes the default; Later writes nothing and the
 * check asks again at the next launch.
 */
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import { readModels, writeDefaultModel } from "./modelChoice";
import { modelTerm } from "../vocabulary";

/** One model the card offers. */
export interface OfferedModel {
  alias: string;
  label: string;
}

/** What the card shows. */
export interface DefaultModelState {
  /** The default Vibe no longer offers, or `null` (no card). */
  gone: OfferedModel | null;
  models: readonly OfferedModel[];
  saving: boolean;
  /** Why the last save failed, in the core's words. */
  error: string | null;
}

const CLOSED: DefaultModelState = { gone: null, models: [], saving: false, error: null };

function goneAlias(result: unknown): string | null {
  if (typeof result !== "object" || result === null) return null;
  const gone = (result as { gone?: unknown }).gone;
  return typeof gone === "string" && gone !== "" ? gone : null;
}

/** Creates the controller. */
export function createDefaultModel(deps: { core: CoreController }) {
  const { core } = deps;
  const store = createStore<DefaultModelState>(CLOSED);

  /** Asks the core; opens the card when it names a default that is gone. */
  const check = async (): Promise<void> => {
    try {
      const answer = await core.run({ type: "checkDefaultModel" });
      const alias = answer.type === "reply" ? goneAlias(answer.result) : null;
      if (alias === null) return;
      const list = await readModels(core);
      const models = list.models
        .filter((m) => m.alias !== alias)
        .map((m) => ({ alias: m.alias, label: m.label }));
      if (models.length === 0) return;
      store.set(() => ({ ...CLOSED, gone: { alias, label: modelTerm(alias).name }, models }));
    } catch {
      // Settings still offers the default model; a failed check never blocks the window.
    }
  };

  /** Saves `alias` as the default for new chats and closes the card. Returns false when it failed. */
  const pick = async (alias: string): Promise<boolean> => {
    const model = store.get().models.find((m) => m.alias === alias);
    if (model === undefined) return false;
    store.set((s) => ({ ...s, saving: true, error: null }));
    try {
      await writeDefaultModel(core, alias);
      store.set(() => CLOSED);
      return true;
    } catch (thrown) {
      const error = thrown instanceof Error ? thrown.message : String(thrown);
      store.set((s) => ({ ...s, saving: false, error }));
      return false;
    }
  };

  /** Closes the card without a change. */
  const later = (): void => {
    store.set(() => CLOSED);
  };

  /** Runs the check each time the server reaches running; returns the unsubscribe. */
  const watch = (): (() => void) => {
    let running = false;
    const seen = () => {
      const now = core.store.get().server.phase === "running";
      if (now && !running) void check();
      running = now;
    };
    seen();
    return core.store.subscribe(seen);
  };

  return { store, check, pick, later, watch } as const;
}

/** The default-model controller. */
export type DefaultModel = ReturnType<typeof createDefaultModel>;
