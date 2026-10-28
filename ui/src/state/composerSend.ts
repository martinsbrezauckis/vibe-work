/**
 * Sending from the composer: a turn, a steer into the running turn, a queued turn, a `!cmd`
 * shell command, stop and retry. Every call goes through the core (`core.run`), so the gate checks it.
 * The UI never builds raw JSON-RPC.
 * Depends on: coreStore (`CoreController`), composerStore, the composer model, protocol types. Called by:
 * createComposer, the composer views (through it), the `ui/composer` route.
 * Invariants:
 * - Content taken from the composer is restored when a send fails (unless the user already typed
 *   something new). Separate button requests never take or restore composer content.
 * - Each message gets its anchor id (`clientUserMessageId` / queue `entryId`) and an
 *   idempotency key when it is first sent. When the send fails in transport (`timeout`,
 *   `serverUnavailable`, `internal`), the keys are kept and reused if the SAME content is sent again.
 *   Vibe honours the key on `session/turn/enqueue` (a repeat answers the original
 *   item). Before resending a `turn/start` / `turn/steer` the history is checked for our anchor
 *   id: if the message is there, the first send arrived and nothing is sent again. Any other failure
 *   drops the keys.
 * - `!cmd` runs only from text typed in the box, by the user or a Debug API client (`model/message`
 *   `classify`; the Debug API acts as the user, and the core supplies its confirmation); a call the
 *   gate still wants confirmed (the window's own dialog) is asked for here, like the session delete flow.
 */
import type { CommandResult, JsonValue } from "../protocol/core";
import { type CoreCallError, toCallError } from "../protocol/client/types";
import {
  asJson,
  classify,
  displayContent,
  queueEntries,
  retryPrompt,
  turnContent,
} from "../features/composer/model/message";
import { runningTurnId, waitingForInput } from "../features/composer/model/session";
import {
  type ComposerStore,
  type SentContent,
  contentKey,
  takeContent,
  isPluginReloadFailure,
} from "./composerStore";
import type { CoreController } from "./coreStore";
import { cardIdOf, hostOf } from "../features/ask/waiting";

/** How a message leaves: a new turn, into the running turn, or after it. */
export type SendMode = "send" | "steer" | "queue";

/** Failures after which Vibe may have received the request: keep the keys for an identical resend. */
const MAYBE_DELIVERED = new Set(["timeout", "serverUnavailable", "internal"]);

/** Random hex for ids and keys (`crypto.getRandomValues`, available in the webview and in jsdom). */
function randomHex(bytes: number): string {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return [...buf].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** What `send` reports to its caller (the route and tests read it). */
export type SendOutcome =
  | { ok: true; kind: "turn" | "steer" | "queue" | "shell" | "command"; result?: CommandResult }
  | { ok: false; kind: string; message: string };

/** The send-side actions. `runCommand` handles `/name` (composerSession). */
export function createSendActions(deps: {
  core: CoreController;
  composer: ComposerStore;
  runCommand: (sessionId: string, name: string, args: string) => Promise<SendOutcome>;
  /** Shared by the user's and the Debug API's action sets, so their sends of one session stay in order. */
  chains: Map<string, Promise<unknown>>;
  /** Readiness can change while a send waits for lazy loading or its previous send. */
  keyGuard?: (() => string | null) | undefined;
  /**
   * Called after a turn this window sent was accepted: the session's channel is open then, so the
   * caller reads the runtime (access mode, model) for the composer chips without opening a channel just for that.
   */
  afterTurn: (sessionId: string) => void;
  /** Apply new-session model preferences before submitting; a failed pick leaves the chat sendable. */
  beforeTurn: (sessionId: string) => Promise<boolean>;
}) {
  const { core, composer, runCommand, chains, afterTurn } = deps;
  const entry = (id: string) => core.store.get().sessions[id];
  const running = (id: string): string | null => {
    const e = entry(id);
    return runningTurnId(e?.state ?? null, e?.session);
  };
  /** True when the session's history holds a user entry with our anchor id (the send arrived). */
  const delivered = (id: string, anchor: string): boolean => {
    const history = entry(id)?.state?.["history"];
    return (
      Array.isArray(history) &&
      history.some(
        (e) => typeof e === "object" && e !== null && !Array.isArray(e) && e["id"] === anchor,
      )
    );
  };
  const call = (method: string, params: Record<string, JsonValue>, token?: string) =>
    core.run({
      type: "call",
      method,
      params,
      ...(token === undefined ? {} : { confirmationToken: token }),
    });

  /** Runs `method` and, when the gate wants the native confirmation (Debug API origin), asks for it. */
  const confirmed = async (method: string, params: Record<string, JsonValue>) => {
    try {
      return await call(method, params);
    } catch (thrown) {
      const error = toCallError(thrown);
      if (error.error.code !== "confirmationRequired") throw error;
      const answer = await core.run({ type: "requestConfirmation", method, params });
      if (answer.type !== "confirmation") throw error;
      return call(method, params, answer.token);
    }
  };

  const fail = (sessionId: string, kind: string, error: CoreCallError): SendOutcome => {
    const blocked = deps.keyGuard?.() ?? null;
    if (blocked !== null || error.error.code === "keyService")
      return { ok: false, kind: "key", message: blocked ?? error.error.message };
    if (error.error.code !== "confirmationDeclined") composer.note(sessionId, error.error.message);
    return { ok: false, kind, message: error.error.message };
  };

  const shell = async (
    sessionId: string,
    command: string,
    text: string,
    by: SentContent["by"],
  ): Promise<SendOutcome> => {
    const pluginsHeld = composer.store.get().pluginReloadFailed[sessionId] === true;
    try {
      await confirmed("session/shellCommand", { sessionId, command });
      composer.note(sessionId, null);
      return { ok: true, kind: "shell" };
    } catch (thrown) {
      const error = toCallError(thrown);
      composer.restore(
        sessionId,
        text,
        [],
        [],
        false,
        by,
        pluginsHeld || isPluginReloadFailure(error.error.message),
      );
      return fail(sessionId, "shell", error);
    }
  };

  /** The last send of each session: sends of one session go out one after the other, in key order. */

  /**
   * Sends content already taken from the composer (`takeContent`), after the previous send of the same
   * session (never dropped, never reordered).
   */
  const sendContent = (
    sessionId: string,
    wanted: SendMode,
    content: SentContent,
  ): Promise<SendOutcome> => {
    const previous = chains.get(sessionId) ?? Promise.resolve();
    const next = previous.then(() => deliver(sessionId, wanted, content));
    chains.set(
      sessionId,
      next.catch(() => undefined),
    );
    return next;
  };

  /** Sends the composer content of `sessionId` the way `mode` says (see the module doc). */
  const send = (sessionId: string, wanted: SendMode): Promise<SendOutcome> => {
    const content = takeContent(composer, sessionId);
    return content === null
      ? Promise.resolve({ ok: false, kind: "empty", message: "Nothing to send." })
      : sendContent(sessionId, wanted, content);
  };

  const deliver = async (
    sessionId: string,
    wanted: SendMode,
    content: SentContent,
  ): Promise<SendOutcome> => {
    const { text, images } = content;
    let context = content.context;
    if (composer.store.get().compacting[sessionId] === true) {
      if (content.restoreOnFailure !== false)
        composer.restore(sessionId, text, context, images, content.literal, content.by);
      composer.note(sessionId, "Context compaction is running. Stop it or wait before sending.");
      return { ok: false, kind: "busy", message: "Context compaction is running." };
    }
    const blocked = deps.keyGuard?.() ?? null;
    if (blocked !== null) {
      if (content.restoreOnFailure !== false)
        composer.restore(sessionId, text, context, images, content.literal, content.by);
      return { ok: false, kind: "key", message: blocked };
    }
    const s = composer.store.get();
    const intent = classify(text, context.length, images.length, content.literal);
    if (intent.kind === "empty") return { ok: false, kind: "empty", message: "Nothing to send." };
    if (intent.kind === "shell") return shell(sessionId, intent.command, text, content.by);
    if (intent.kind === "command") {
      if (
        (content.literal && content.restoreOnFailure !== false) ||
        context.length > 0 ||
        images.length > 0
      ) {
        const message = content.literal
          ? "This is an app command. Choose Use as my command before running it."
          : "App commands cannot carry attachments. Remove the attachments before running it.";
        if (content.restoreOnFailure !== false)
          composer.restore(sessionId, text, context, images, content.literal, content.by);
        composer.note(sessionId, message);
        return { ok: false, kind: "command", message };
      }
      const outcome = await runCommand(sessionId, intent.name, intent.args);
      if (!outcome.ok && content.restoreOnFailure !== false)
        composer.restore(sessionId, text, [], [], false, content.by);
      return outcome;
    }
    if (entry(sessionId)?.localOnly === true) await deps.beforeTurn(sessionId);
    try {
      context = await composer.prepareContext(sessionId, context);
      content = { ...content, context };
    } catch (error) {
      if (content.restoreOnFailure !== false)
        composer.restore(sessionId, text, context, images, content.literal, content.by);
      return fail(sessionId, "documents", toCallError(error));
    }
    const turnId = running(sessionId);
    // Separate button requests must not become a steer if the session started working meanwhile.
    if (content.restoreOnFailure === false && turnId !== null)
      return { ok: false, kind: "busy", message: "The agent is working; try when it is done" };
    const voice = content.voice === true;
    const asked: SendMode = turnId === null ? "send" : wanted === "send" ? "steer" : wanted;
    // A voice turn is a queued turn even when the session is idle: only `session/turn/enqueue` carries
    // a context entry, and Vibe starts an idle session's item at once. A steer carries no context.
    const waiting = waitingForInput(entry(sessionId)?.state ?? null, turnId);
    const mode: SendMode =
      turnId !== null && waiting ? "steer" : voice && asked === "send" ? "queue" : asked;
    const key = contentKey(text, context, images);
    const kept = s.resend[sessionId];
    const keys =
      kept !== undefined && kept.content === key
        ? kept
        : {
            content: key,
            idempotencyKey: `vw-idem-${randomHex(12)}`,
            clientUserMessageId: `vw-msg-${randomHex(12)}`,
          };
    if (keys === kept && mode !== "queue" && delivered(sessionId, keys.clientUserMessageId)) {
      composer.setResend(sessionId, null);
      composer.note(sessionId, "That message had reached Vibe already; it was not sent twice.");
      return { ok: true, kind: mode === "send" ? "turn" : mode };
    }
    const display = displayContent(text, context, images, voice && mode !== "steer");
    const base: Record<string, JsonValue> = {
      sessionId,
      idempotencyKey: keys.idempotencyKey,
    };
    composer.setSending(sessionId, true);
    try {
      let result: CommandResult;
      if (mode === "queue") {
        const entries = queueEntries(text, context, images, keys.clientUserMessageId, voice);
        result = await call("session/turn/enqueue", { ...base, entries: asJson(entries) });
      } else {
        const params: Record<string, JsonValue> = {
          ...base,
          message: asJson(turnContent(text, context, images)),
          clientUserMessageId: keys.clientUserMessageId,
          ...(display === null ? {} : { userDisplayContent: asJson(display) }),
        };
        if (mode === "steer" && turnId !== null) params["expectedTurnId"] = turnId;
        result = await call(mode === "steer" ? "turn/steer" : "turn/start", params);
      }
      composer.setResend(sessionId, null);
      composer.note(sessionId, null);
      // Only a turn that started an idle session: a steer or a queued item rides a channel that was read already.
      if (turnId === null) afterTurn(sessionId);
      return {
        ok: true,
        kind: mode === "send" || (voice && asked === "send") ? "turn" : mode,
        result,
      };
    } catch (thrown) {
      const error = toCallError(thrown);
      composer.setResend(sessionId, MAYBE_DELIVERED.has(error.error.code) ? keys : null);
      if (content.restoreOnFailure === false) return fail(sessionId, mode, error);
      if (
        !composer.restore(
          sessionId,
          text,
          context,
          images,
          content.literal,
          content.by,
          s.pluginReloadFailed[sessionId] === true ||
            composer.store.get().pluginReloadFailed[sessionId] === true ||
            isPluginReloadFailure(error.error.message),
        )
      ) {
        if (deps.keyGuard?.() != null || error.error.code === "keyService")
          return fail(sessionId, mode, error);
        composer.note(sessionId, `Not sent (${error.error.message}). Your new text was kept.`);
        return { ok: false, kind: mode, message: error.error.message };
      }
      if (mode === "steer" && error.error.serverCode === "stale_turn") {
        composer.note(
          sessionId,
          "The turn ended before the steer arrived. Press Enter to send it.",
        );
        return { ok: false, kind: mode, message: error.error.message };
      }
      return fail(sessionId, mode, error);
    } finally {
      composer.setSending(sessionId, false);
    }
  };

  return {
    send,
    sendContent,
    /** Stops the chat and its owned work, including cards waiting after its own turn ended. */
    async stop(sessionId: string): Promise<boolean> {
      if (composer.store.get().compacting[sessionId] === true) {
        try {
          await core.run({ type: "stopSession", sessionId });
          composer.setCompacting(sessionId, false);
          // A compaction in flight still finishes after the Stop; a message sent now waits for it and is not lost.
          composer.note(sessionId, "Still compacting. Your next message is sent after that.");
          return true;
        } catch (thrown) {
          fail(sessionId, "stop", toCallError(thrown));
          return false;
        }
      }
      const turnId = running(sessionId);
      const state = core.store.get();
      const waiting = Object.values(state.asks).some(
        (ask) => cardIdOf(ask) !== null && hostOf(state, ask) === sessionId,
      );
      if (turnId === null && !waiting) return false;
      try {
        if (turnId === null) await core.run({ type: "stopSession", sessionId });
        else await core.run({ type: "interruptTurn", sessionId, expectedTurnId: turnId });
        return true;
      } catch (thrown) {
        fail(sessionId, "stop", toCallError(thrown));
        return false;
      }
    },
    /**
     * Retry: Vibe's continue prompt as an injected turn (`/retry`, see `retryPrompt`), optionally with
     * the user's extra instructions.
     */
    async retry(sessionId: string, instructions: string): Promise<SendOutcome> {
      const intent = classify(instructions, 0, 0);
      if (intent.kind === "command") return runCommand(sessionId, intent.name, intent.args);
      const blocked = deps.keyGuard?.() ?? null;
      if (blocked !== null) return { ok: false, kind: "key", message: blocked };
      try {
        const result = await call("turn/start", {
          sessionId,
          injected: true,
          message: [{ type: "text", text: retryPrompt(instructions) }],
        });
        return { ok: true, kind: "turn", result };
      } catch (thrown) {
        return fail(sessionId, "retry", toCallError(thrown));
      }
    },
    /** Saves an edited queued turn: its text blocks are replaced, everything else is kept. */
    async replaceQueued(sessionId: string, queueItemId: string, text: string): Promise<boolean> {
      if (classify(text, 0, 0).kind === "command") {
        composer.note(
          sessionId,
          "App commands run from the composer and cannot be queued. The queued message was kept.",
        );
        return false;
      }
      const state = entry(sessionId)?.state;
      const queue = state?.["turnQueue"];
      const items =
        typeof queue === "object" && queue !== null && !Array.isArray(queue)
          ? queue["items"]
          : null;
      const item = Array.isArray(items)
        ? items.find(
            (i) =>
              typeof i === "object" && i !== null && !Array.isArray(i) && i["id"] === queueItemId,
          )
        : undefined;
      const old =
        typeof item === "object" && item !== null && !Array.isArray(item) ? item["entries"] : null;
      if (!Array.isArray(old)) {
        composer.restoreQueueEdit(sessionId, text);
        composer.note(
          sessionId,
          "This queued message is no longer waiting. Your edit is in the message box.",
        );
        return false;
      }
      const entries = old.map((e) => {
        if (typeof e !== "object" || e === null || Array.isArray(e) || e["role"] !== "user")
          return e;
        const content = Array.isArray(e["content"]) ? e["content"] : [];
        const rest = content.filter(
          (b) =>
            !(typeof b === "object" && b !== null && !Array.isArray(b) && b["type"] === "text"),
        );
        return { ...e, content: [{ type: "text", text }, ...rest] };
      });
      try {
        await call("session/turn/queue/replace", { sessionId, queueItemId, entries });
        composer.setEditing(null);
        return true;
      } catch (thrown) {
        fail(sessionId, "queue", toCallError(thrown));
        return false;
      }
    },
    /** Removes a queued turn before it starts. */
    async removeQueued(sessionId: string, queueItemId: string): Promise<boolean> {
      try {
        await call("session/turn/queue/remove", { sessionId, queueItemId });
        return true;
      } catch (thrown) {
        fail(sessionId, "queue", toCallError(thrown));
        return false;
      }
    },
  };
}

/** The send-side actions. */
export type SendActions = ReturnType<typeof createSendActions>;
