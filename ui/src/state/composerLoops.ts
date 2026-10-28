/**
 * The composer's loops (Vibe's recurring prompts, protocol `loops/*`): the `/loop` command, the list the Loops
 * strip shows and the deletes it asks for. Loops are Vibe's: this file only calls `loops/list`, `loops/create`,
 * `loops/delete` and `loops/clear` through the gate and keeps the last list in the composer store.
 * Depends on: composerStore, toastStore (types), composer/model/loopArgs and loops, protocol/client (`toCallError`).
 * Called by: composerSession (`runCommand` "loop" and the three actions it exposes), the Loops strip through the
 * composer facade (`loadLoops`, `deleteLoop`, `clearLoops`).
 * Invariants:
 * - Nothing is shown as created or deleted before Vibe answered; after every change the list is read again, so
 *   what the strip shows is what Vibe holds (the model can add and cancel loops itself through its `cron` tool).
 * - A read that started before a change is dropped when it ends after it (`epoch`), so an old list never
 *   overwrites a newer one.
 * - The `/loop` note always says that the loop runs by itself and under the session's access mode.
 * - A background read (`loadLoops`) never raises a toast or a note: a session whose channel is closed simply keeps
 *   the list it had.
 */
import type { JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import { LOOP_USAGE, type LoopCommand, parseLoopArgs } from "../features/composer/model/loopArgs";
import {
  type LoopRow,
  createdNote,
  listedNote,
  loopCount,
  loopRows,
  modeClause,
  replyLoop,
  shortPrompt,
} from "../features/composer/model/loops";
import type { SendOutcome } from "./composerSend";
import type { ComposerStore } from "./composerStore";
import type { Toasts } from "./toastStore";

/** What the loop actions need from their surroundings. */
export interface LoopDeps {
  /** One gated core call; resolves to the reply's result. */
  call: (method: string, params: Record<string, JsonValue>) => Promise<JsonValue | null>;
  composer: ComposerStore;
  toasts: Toasts;
  /** The session's access mode: its id and its label, for the note. */
  mode: (sessionId: string) => { id: string | null; label: string | null };
  /** Epoch seconds; replaced in tests. */
  now?: () => number;
}

/** True when the failure is a refusal of the user's own input (Vibe's `invalid_params`), not a broken connection. */
function refusedInput(thrown: unknown): boolean {
  const { error } = toCallError(thrown);
  return error.code === "invalidParams" || error.serverCode === "invalid_params";
}

/** The `count` of a `loops/clear` reply, 0 when it has none. */
function clearedCount(result: JsonValue | null): number {
  const count =
    typeof result === "object" && result !== null && !Array.isArray(result)
      ? result["count"]
      : undefined;
  return typeof count === "number" ? count : 0;
}

/** Builds the loop actions. */
export function createLoopActions(deps: LoopDeps) {
  const { call, composer, toasts } = deps;
  const now = deps.now ?? ((): number => Date.now() / 1000);
  const epoch = new Map<string, number>();
  const reading = new Map<string, Promise<void>>();
  const changed = (sessionId: string): void => {
    epoch.set(sessionId, (epoch.get(sessionId) ?? 0) + 1);
  };

  /** Reads the list and keeps it, unless a change happened while the read was out. Throws when the read fails. */
  const refresh = async (sessionId: string): Promise<LoopRow[]> => {
    const started = epoch.get(sessionId) ?? 0;
    const rows = loopRows(await call("loops/list", { sessionId }));
    if ((epoch.get(sessionId) ?? 0) === started) composer.setLoops(sessionId, rows);
    return rows;
  };
  const done = (): SendOutcome => ({ ok: true, kind: "command" });
  const invalid = (sessionId: string, message: string): SendOutcome => {
    composer.note(sessionId, message);
    return { ok: false, kind: "command", message };
  };
  const fail = (sessionId: string, message: string): SendOutcome => {
    composer.note(sessionId, message);
    toasts.push({ kind: "error", bellKind: "failure", text: message });
    return { ok: false, kind: "command", message };
  };

  const actions = {
    /**
     * Reads the session's loops into the store (`loops/list`). Once per session unless `force`; concurrent calls
     * without `force` share one read, a forced one always asks again. Silent on failure.
     */
    loadLoops(sessionId: string, force = false): Promise<void> {
      if (!force) {
        if (composer.store.get().loops[sessionId] !== undefined) return Promise.resolve();
        const running = reading.get(sessionId);
        if (running !== undefined) return running;
      }
      const read = refresh(sessionId).then(
        () => undefined,
        () => undefined,
      );
      reading.set(sessionId, read);
      void read.finally(() => {
        if (reading.get(sessionId) === read) reading.delete(sessionId);
      });
      return read;
    },

    /** Deletes one loop (`loops/delete`) and reads the list again. Returns whether Vibe deleted it. */
    async deleteLoop(sessionId: string, loopId: string): Promise<boolean> {
      changed(sessionId);
      try {
        await call("loops/delete", { sessionId, loopId });
        await actions.loadLoops(sessionId, true);
        return true;
      } catch (thrown) {
        const error = toCallError(thrown).error;
        if (error.code !== "accessModeUnverified")
          toasts.push({ kind: "error", bellKind: "failure", text: error.message });
        // The loop may be gone already (it was cancelled elsewhere): show what Vibe holds now.
        await actions.loadLoops(sessionId, true);
        return false;
      }
    },

    /** Deletes every loop of the session (`loops/clear`). Returns how many Vibe deleted, or `null` on failure. */
    async clearLoops(sessionId: string): Promise<number | null> {
      changed(sessionId);
      try {
        const count = clearedCount(await call("loops/clear", { sessionId }));
        await actions.loadLoops(sessionId, true);
        return count;
      } catch (thrown) {
        const error = toCallError(thrown).error;
        if (error.code !== "accessModeUnverified")
          toasts.push({ kind: "error", bellKind: "failure", text: error.message });
        await actions.loadLoops(sessionId, true);
        return null;
      }
    },

    /** Runs `/loop <args>`: add, list or cancel. Never sends a turn. */
    async runLoopCommand(sessionId: string, args: string): Promise<SendOutcome> {
      const command: LoopCommand | null = parseLoopArgs(args);
      if (command === null) return invalid(sessionId, LOOP_USAGE);
      try {
        switch (command.kind) {
          case "list": {
            const rows = await refresh(sessionId);
            composer.setLoopsOpen(sessionId, rows.length > 0);
            composer.note(sessionId, listedNote(rows.length));
            return done();
          }
          case "create": {
            if (command.prompt === "")
              return invalid(sessionId, "Say what to repeat: /loop 5m <what to repeat>.");
            changed(sessionId);
            const created = replyLoop(
              await call("loops/create", {
                sessionId,
                interval: command.interval,
                prompt: command.prompt,
              }),
            );
            await actions.loadLoops(sessionId, true);
            // The read after a change can fail on its own; Vibe's answer to the create still counts.
            const held = composer.store.get().loops[sessionId] ?? [];
            if (created !== null && !held.some((row) => row.id === created.id))
              composer.setLoops(
                sessionId,
                [...held, created].sort((a, b) => a.nextFireAt - b.nextFireAt),
              );
            composer.setLoopsOpen(sessionId, true);
            const mode = deps.mode(sessionId);
            // Vibe's answer is the loop; without it the list is what Vibe holds, and the note says so.
            composer.note(
              sessionId,
              created === null
                ? "Loop added: see the Loops list above the message box."
                : createdNote(created, now(), modeClause(mode.id, mode.label)),
            );
            return done();
          }
          case "cancel": {
            if (command.id === "")
              return invalid(sessionId, "Say which loop: /loop cancel <id> or /loop cancel all.");
            changed(sessionId);
            const gone = replyLoop(await call("loops/delete", { sessionId, loopId: command.id }));
            await actions.loadLoops(sessionId, true);
            if ((composer.store.get().loops[sessionId] ?? []).length === 0)
              composer.setLoopsOpen(sessionId, false);
            composer.note(
              sessionId,
              `Loop ${command.id} deleted${gone === null ? "" : `: "${shortPrompt(gone.prompt)}"`}.`,
            );
            return done();
          }
          case "cancelAll": {
            changed(sessionId);
            const count = clearedCount(await call("loops/clear", { sessionId }));
            await actions.loadLoops(sessionId, true);
            composer.setLoopsOpen(sessionId, false);
            composer.note(
              sessionId,
              count > 0 ? `${loopCount(count)} deleted.` : "There were no loops to delete.",
            );
            return done();
          }
        }
      } catch (thrown) {
        const error = toCallError(thrown).error;
        const message = error.message;
        if (error.code === "accessModeUnverified") return invalid(sessionId, message);
        return refusedInput(thrown)
          ? invalid(sessionId, `${message} ${LOOP_USAGE}`)
          : fail(sessionId, message);
      }
    },
  };
  return actions;
}
