/**
 * The composer's actions, loaded on first use (`composer.ts` holds the facade): send, steer,
 * queue, `!cmd`, stop, retry, the session controls (model, access mode, skills, `/` commands), rewind and
 * branch, and attaching images. Kept out of the main bundle (the main chunk was 528 kB).
 * Depends on: composerSend, composerSession, composerRewind, the image model, coreStore, composerStore,
 * shellStore, sessionActions, toastStore. Called by: `composer.ts` (dynamic import).
 * Invariants: the facade caches the user's, Debug API's and trusted button-click action sets;
 * `origin: "debug-api"` serves UI-control requests, while `origin: "window"` preserves a native click
 * through lazy loading and a queued send even when an unrelated Debug API read is pending;
 * they share the per-session send order.
 */
import { fileBytes, imageFromBytes, isImage } from "../features/composer/model/images";
import { createRewindActions } from "./composerRewind";
import { createSendActions } from "./composerSend";
import { createSessionControls } from "./composerSession";
import type { ComposerStore } from "./composerStore";
import type { CoreController } from "./coreStore";
import type { SessionActions } from "./sessionActions";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import type { ComposerInitialization } from "./composerDefaults";

/** A view of `core` whose `run` keeps the chosen origin through deferred work. */
function lowered(core: CoreController, origin: "debug-api" | "window"): CoreController {
  const view = Object.create(core) as CoreController & { run: CoreController["run"] };
  view.run = (command) => core.run(command, { origin });
  return view;
}

/** Builds the actions. */
export function createComposerActions(deps: {
  core: CoreController;
  composer: ComposerStore;
  shell: Shell;
  toasts: Toasts;
  sessions: SessionActions | null;
  /** Explicit origins survive lazy loading and per-session send ordering. */
  origin: "ui" | "debug-api" | "window";
  chains: Map<string, Promise<unknown>>;
  initialization: ComposerInitialization;
  /** Rechecked after lazy loading and queued sends; reads key metadata only. */
  keyGuard?: (() => string | null) | undefined;
  /** Opens the Tools page (`/mcp`, `/connectors`). */
  openTools?: (() => void) | undefined;
}) {
  const { composer, shell, toasts, sessions, chains } = deps;
  // The origin is bound when the set is built, never read from a global at call time.
  const core: CoreController = deps.origin === "ui" ? deps.core : lowered(deps.core, deps.origin);
  const rewind = createRewindActions({ core, composer, shell, toasts });
  let send: ReturnType<typeof createSendActions> | null = null;
  const controls = createSessionControls({
    core,
    composer,
    shell,
    toasts,
    sessions,
    rewind,
    trustedAccessPick: deps.origin === "window",
    initialization: deps.initialization,
    openTools: deps.openTools,
    send: () => {
      if (send === null) throw new Error("composer send actions are not ready");
      return send;
    },
  });
  send = createSendActions({
    core,
    composer,
    runCommand: (id, name, args) => controls.runCommand(id, name, args),
    chains,
    keyGuard: deps.keyGuard,
    beforeTurn: (id) => controls.prepareSession(id),
    // UX-05: the channel is open after a turn went out, so the chips can name the real mode and model.
    afterTurn: (id) => {
      if (core.store.get().sessions[id]?.runtime === undefined) void controls.loadRuntime(id);
    },
  });
  return {
    ...send,
    ...controls,
    ...rewind,
    /** Attaches image files the user chose; refused ones are named in one toast. Returns the count added. */
    async attachFiles(sessionId: string, files: readonly File[]): Promise<number> {
      let added = 0;
      const refused: string[] = [];
      for (const file of files) {
        const already = (composer.store.get().images[sessionId] ?? []).length;
        const result = imageFromBytes(
          file.name,
          await fileBytes(file),
          composer.nextId("img"),
          already,
        );
        if (isImage(result)) {
          composer.addImage(
            sessionId,
            deps.origin === "debug-api" ? { ...result, by: "debug-api" } : result,
          );
          added += 1;
        } else {
          refused.push(`${result.name}: ${result.reason}`);
        }
      }
      if (refused.length > 0) {
        toasts.push({
          kind: "warn",
          bellKind: "failure",
          text: `Not attached: ${refused.join(" · ")}`,
        });
      }
      return added;
    },
  };
}

/** The loaded actions. */
export type ComposerActions = ReturnType<typeof createComposerActions>;
