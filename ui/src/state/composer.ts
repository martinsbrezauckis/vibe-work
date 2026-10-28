/**
 * The composer as one object: its store (main bundle: drafts must load at start) plus a
 * facade over the actions, whose implementation (`composerImpl.ts`) loads on first use so the main
 * bundle does not grow with it. Button messages use `sendMessage`, separately from drafts and attachments.
 * Two entry points for other features stay synchronous: `insertContext`
 * ("Point Vibe at this", quotes, page comments) and `closeRewind`.
 * Depends on: composerStore, coreStore, shellStore, sessionActions, toastStore; `composerImpl` lazily.
 * Called by: createApp (one per app), the composer views, the user-row buttons, the `ui/composer` route,
 * the palette entries.
 * Invariants: without a core `actions` is `null`; inserted context is always a chip (never draft text),
 * so it can never become a command; a deleted session's draft and chips are dropped.
 */
import type { ContextItem } from "../features/composer/model/message";
import { selectedModel } from "../features/composer/model/models";
import type { ComposerActions } from "./composerImpl";
import { runningTurnId } from "../features/composer/model/session";
import { type SentContent, createComposerStore, takeContent } from "./composerStore";
import { requestComposerFocus } from "./composerFocus";
import { NEW_CHAT } from "./newChatKey";
import type { SendMode, SendOutcome } from "./composerSend";
import type { CoreController } from "./coreStore";
import type { SessionActions } from "./sessionActions";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import type { ComposerInitialization } from "./composerDefaults";

/**
 * What was open on the New chat screen (terminals, browser, files) follows the session its first message starts
 * (`state/newChatDraft.ts`): told before the start, and after it with the started id (`null`: nothing moves).
 */
export interface NewChatHandOver {
  expectStart(): void;
  finishStart(started: string | null): Promise<void>;
}

/** The asynchronous actions the facade forwards to the loaded implementation. */
const DEFERRED = [
  "stop",
  "retry",
  "retryPluginRefresh",
  "replaceQueued",
  "removeQueued",
  "loadRuntime",
  "prepareSession",
  "loadConfig",
  "loadSkills",
  "loadLoops",
  "deleteLoop",
  "clearLoops",
  "pickModel",
  "setAccess",
  "runCommand",
  "openRewind",
  "confirmRewind",
  "branch",
  "attachFiles",
] as const satisfies readonly (keyof ComposerActions)[];

type Deferred = Pick<ComposerActions, (typeof DEFERRED)[number]>;

/**
 * How the new-chat view starts its session. The view installs one when it shows the "Work in a new copy"
 * section (the copy is made by `worktrees.start`); without one the session starts in the chip's folder itself
 * (`sessions.startIn`, the path "New session here" takes: no trust grant).
 */
export interface NewChatStarter {
  /** Why Send cannot start now (for example a copy without a branch name), or `null`. */
  refuse(): string | null;
  /** Starts the session; resolves to its id, or `null` when it did not start (the failure was shown). */
  start(): Promise<string | null>;
}

/**
 * Runs before a message of `sessionId` goes out: `null` when there is nothing to do (the send starts at once), else
 * the work, answering the note to show after the send (or null).
 */
export type BeforeSend = (
  sessionId: string,
  origin: "ui" | "debug-api",
) => Promise<string | null> | null;

/** What a send from the new-chat view resolved to: the usual outcome, plus the session it started. */
export type NewChatOutcome = SendOutcome & { started?: string };

/** Creates the composer. `persist` keeps drafts in `localStorage`. */
export function createComposer(deps: {
  core: CoreController | null;
  shell: Shell;
  toasts: Toasts;
  sessions: SessionActions | null;
  persist: boolean;
  /** Metadata-only key guard, shared by text, button, command and spoken submissions. */
  keyGuard?: () => string | null;
  /** Opens the Tools page (`/mcp`, `/connectors`); bound late by the app (connectors are made after the composer). */
  openTools?: () => void;
}) {
  const { core } = deps;
  const composer = createComposerStore(deps.persist);
  if (core === null) {
    return {
      ...composer,
      core: null,
      actions: null,
      debugApi: null,
      // No core: nothing is ever sent, so there is nothing to hold back.
      setSendGuard: (): void => undefined,
      setContentTaken: (): void => undefined,
      setContentSettled: (): void => undefined,
      setNewChatStarter: (): void => undefined,
      setNewChatHandOver: (): void => undefined,
      setVoiceTurns: (): void => undefined,
      setBeforeSend: (): void => undefined,
    } as const;
  }

  core.listen((event) => {
    // Messages queued behind a stopped or failed turn: never sent (the core took them out of Vibe's queue), back in
    // the composer for the user to send, change or drop. No toast, no dialog.
    if (event.type === "queueReturned") {
      if (composer.takeBackQueued(event.sessionId, event.items))
        requestComposerFocus(event.sessionId);
      return;
    }
    if (event.type === "alert" && event.kind === "pluginReloadFailed" && event.sessionId !== null) {
      composer.pluginReloadRefused(event.sessionId, event.message);
    }
  });
  const chains = new Map<string, Promise<unknown>>();
  let menuRequest = 0;
  const initialization: ComposerInitialization = {
    ready: new Set(),
    pending: new Map(),
    choices: new Map(),
  };
  // A document admission guard and attachment send lifecycle, installed after the composer is created.
  let sendGuard: ((sessionId: string) => string | null) | null = null;
  let contentTaken: ((sessionId: string, context: readonly ContextItem[]) => void) | null = null;
  let contentSettled:
    ((sessionId: string, context: readonly ContextItem[], sent: boolean) => void) | null = null;
  // The new-chat view's starter while it shows the copy section; `null`: start in the folder itself.
  let newChatStarter: NewChatStarter | null = null;
  let newChatHandOver: NewChatHandOver | null = null;
  // Whether a session is in voice chat (its turns then go out as voice turns); set by the voice controller.
  let voiceTurns: ((sessionId: string) => boolean) | null = null;
  // Runs before a message goes out (the files controller saves edited open files); answers a note or null.
  let beforeSend: BeforeSend | null = null;
  /**
   * Runs the before-send step, then `send`; its note shows after a successful send. Never blocks the send. With
   * nothing to save the send starts at once (no extra wait); a save in between is tracked, so a Debug API request
   * that sent the message stays served (and its origin kept) until the message went out.
   */
  const afterSaving = (
    sessionId: string,
    origin: "ui" | "debug-api",
    send: () => Promise<SendOutcome>,
  ): Promise<SendOutcome> => {
    let pending: Promise<string | null> | null = null;
    try {
      pending = beforeSend?.(sessionId, origin) ?? null;
    } catch {
      // Saving first is a convenience; the message goes either way.
    }
    if (pending === null) return send();
    const saving = pending;
    return core.trackAction(
      (async () => {
        const note = await saving.catch(() => null);
        const outcome = await send();
        if (note !== null && outcome.ok) composer.note(sessionId, note);
        return outcome;
      })(),
    );
  };
  const asVoice = (sessionId: string, content: SentContent): SentContent =>
    voiceTurns?.(sessionId) === true ? { ...content, voice: true } : content;
  const loading: Partial<Record<"ui" | "debug-api" | "window", Promise<ComposerActions>>> = {};
  /** The implementation bound to `origin`, loaded once per origin. */
  const load = (origin: "ui" | "debug-api" | "window"): Promise<ComposerActions> => {
    loading[origin] ??= import("./composerImpl").then(({ createComposerActions }) =>
      createComposerActions({ ...deps, core, composer, origin, chains, initialization }),
    );
    return loading[origin];
  };

  /** The action set for one origin (`debug-api`: the `ui/composer` route; the origin is fixed here). */
  const facade = (origin: "ui" | "debug-api") => {
    const deferred = Object.fromEntries(
      DEFERRED.map((name) => [
        name,
        (...args: unknown[]) =>
          core.trackAction(
            load(origin).then((impl) => (impl[name] as (...a: unknown[]) => unknown)(...args)),
          ),
      ]),
    ) as unknown as Deferred;
    /**
     * The new-chat view's Send (the session exists only from the first send). Without a folder
     * nothing is taken: the chip lights up and the note asks for one, the text stays. Otherwise the content is taken
     * at the key press, the session starts in the chip's folder (or its copy), the held access mode and model are
     * applied, and only then is the content sent as the first turn. Nothing typed is ever lost: a start that fails
     * puts the content back here; picks that fail (Full access declined) leave it unsent in the new session's box.
     * Like any send, the first turn saves the edited open files first (the before-send step): the files the New chat
     * screen had open are the session's by then (the hand-over), so they are on disk before the turn starts.
     */
    const sendNewChat = async (given?: SentContent): Promise<NewChatOutcome> => {
      const keyBlocked = deps.keyGuard?.() ?? null;
      if (keyBlocked !== null) return { ok: false, kind: "key", message: keyBlocked };
      const shellNow = deps.shell.store.get();
      const folder = shellNow.newFolder;
      const refuse = (kind: string, message: string): NewChatOutcome => {
        composer.note(NEW_CHAT, message);
        return { ok: false, kind, message };
      };
      if (folder === null) {
        composer.holdNewChat({ askFolder: true });
        return refuse(
          "folder",
          "Choose a folder first: the chat starts in it. Your text stays here.",
        );
      }
      if (composer.store.get().sending[NEW_CHAT] === true) {
        return { ok: false, kind: "busy", message: "The chat is starting." };
      }
      const refused = newChatStarter?.refuse() ?? null;
      if (refused !== null) return refuse("copy", refused);
      const taken = given ?? takeContent(composer, NEW_CHAT);
      if (taken === null) return { ok: false, kind: "empty", message: "Nothing to send." };
      const content = asVoice(NEW_CHAT, taken);
      contentTaken?.(NEW_CHAT, content.context);
      const held = composer.store.get().newChat;
      // Whose action the start is: the Debug API's when it called, wrote any of the content, held a pick, or chose
      // the folder. Applied after the client's request has ended, the core runs it as the window, so the user
      // sees the normal confirmation for Full access.
      const bound =
        origin === "debug-api" ||
        content.by === "debug-api" ||
        held.by === "debug-api" ||
        shellNow.newFolderBy === "debug-api"
          ? "debug-api"
          : "ui";
      composer.note(NEW_CHAT, null);
      composer.setSending(NEW_CHAT, true);
      // Only a started session takes what was open (once); a failed start moves nothing.
      let started: string | null = null;
      let handed = false;
      let sent = false;
      const handOver = async (): Promise<void> => {
        if (handed) return;
        handed = true;
        await newChatHandOver?.finishStart(started);
      };
      try {
        const impl = await load(bound);
        await impl.loadConfig();
        newChatHandOver?.expectStart();
        started =
          newChatStarter === null
            ? ((await deps.sessions?.startIn(folder)) ?? null)
            : await newChatStarter.start();
        if (started === null) {
          const back = composer.restore(
            NEW_CHAT,
            content.text,
            content.context,
            content.images,
            content.literal,
            content.by,
          );
          // Typed again meanwhile: the new text stays in the box and the unsent one is quoted in the note.
          return refuse(
            "start",
            back
              ? "The chat did not start. Your text is still here."
              : `The chat did not start. Not sent: ${content.text}`,
          );
        }
        // The new session is selected now (the start selects it): its composer takes the keyboard.
        requestComposerFocus(started);
        await impl.prepareSession(started, true, held);
        let picked = true;
        // What was open on the New chat screen is the session's before its first message goes out.
        await handOver();
        composer.resetNewChat();
        if (held.mode !== null) {
          // Keep the native access choice's consent across session creation, without promoting API input.
          const access = held.trustedAccessPick && bound === "ui" ? await load("window") : impl;
          picked = await access.setAccess(started, held.mode);
        }
        if (!picked) {
          composer.restore(
            started,
            content.text,
            content.context,
            content.images,
            content.literal,
            content.by,
          );
          const message =
            "Not sent yet: the access mode you chose was not set. Your message is in the box; send it when ready.";
          composer.note(started, message);
          return { ok: false, kind: "picks", message, started };
        }
        // The open files moved to the session with the hand-over above: save them, then send the first turn.
        const sessionId = started;
        const outcome = await afterSaving(sessionId, bound, () =>
          impl.sendContent(sessionId, "send", content),
        );
        sent = outcome.ok;
        return { ...outcome, started };
      } finally {
        await handOver();
        contentSettled?.(started ?? NEW_CHAT, content.context, sent);
        composer.setSending(NEW_CHAT, false);
      }
    };
    /** The new-chat view's picks are held, not sent (there is no session yet). */
    const holdPick = (patch: Parameters<typeof composer.holdNewChat>[0]): Promise<boolean> => {
      composer.setMenu(null);
      // Provenance: once a Debug API client held a pick, the picks are its (a later window pick keeps the mark).
      const by = composer.store.get().newChat.by === "debug-api" ? "debug-api" : origin;
      composer.holdNewChat({ ...patch, by });
      return Promise.resolve(true);
    };
    return {
      ...deferred,
      async openMenu(sessionId: string, kind: "model" | "access" | null): Promise<void> {
        const request = ++menuRequest;
        composer.setMenu(kind === null ? null : { sessionId, kind });
        if (kind === null) return;
        // Attached chats receive Vibe's runtime updates. The new-chat menu reads the lobby.
        if (kind === "model" && sessionId === NEW_CHAT) {
          const loaded = await deferred.loadConfig();
          if (!loaded && request === menuRequest) composer.setMenu(null);
        }
      },
      setAccess(sessionId: string, agentName: string, trustedClick = false): Promise<boolean> {
        if (sessionId === NEW_CHAT)
          return holdPick({
            mode: agentName,
            trustedAccessPick: origin === "ui" && trustedClick,
          });
        if (origin === "ui" && trustedClick)
          return core.trackAction(
            load("window").then((impl) => impl.setAccess(sessionId, agentName)),
          );
        return deferred.setAccess(sessionId, agentName);
      },
      pickModel(
        sessionId: string,
        modelAlias?: string,
        reasoningEffort?: string,
      ): Promise<boolean> {
        if (sessionId === NEW_CHAT) {
          return holdPick({
            ...(modelAlias === undefined ? {} : { alias: modelAlias }),
            ...(modelAlias === undefined
              ? {}
              : selectedModel(composer.store.get().config ?? undefined, modelAlias)),
            ...(reasoningEffort === undefined ? {} : { effort: reasoningEffort }),
          });
        }
        return deferred.pickModel(sessionId, modelAlias, reasoningEffort);
      },
      // Nothing to read for a session that does not exist yet (the view shows the configured models).
      loadRuntime(sessionId: string): Promise<boolean> {
        return sessionId === NEW_CHAT ? Promise.resolve(false) : deferred.loadRuntime(sessionId);
      },
      prepareSession(sessionId: string): Promise<boolean> {
        return sessionId === NEW_CHAT ? deferred.loadConfig() : deferred.prepareSession(sessionId);
      },
      loadSkills(sessionId: string): Promise<void> {
        return sessionId === NEW_CHAT ? Promise.resolve() : deferred.loadSkills(sessionId);
      },
      // A chat that has not started has no loops.
      loadLoops(sessionId: string, force = false): Promise<void> {
        return sessionId === NEW_CHAT ? Promise.resolve() : deferred.loadLoops(sessionId, force);
      },
      runCommand(sessionId: string, name: string, args: string): Promise<SendOutcome> {
        const blocked = deps.keyGuard?.() ?? null;
        if (blocked !== null) return Promise.resolve({ ok: false, kind: "key", message: blocked });
        if (sessionId === NEW_CHAT) {
          const message = `/${name} works once the chat has started: send a first message.`;
          composer.note(NEW_CHAT, message);
          return Promise.resolve({ ok: false, kind: "command", message });
        }
        return deferred.runCommand(sessionId, name, args);
      },
      /**
       * Sends the composer content the way `mode` says. The content is taken synchronously, at the key
       * press, before the action code is awaited (a second key press then sends the next text). Content
       * a Debug API client wrote goes out with the Debug API origin whoever sends it (provenance).
       */
      send(sessionId: string, mode: SendMode): Promise<NewChatOutcome> {
        const keyBlocked = deps.keyGuard?.() ?? null;
        if (keyBlocked !== null)
          return Promise.resolve({ ok: false, kind: "key", message: keyBlocked });
        if (sessionId === NEW_CHAT) return sendNewChat();
        const blocked = sendGuard?.(sessionId) ?? null;
        if (blocked !== null) {
          composer.note(sessionId, blocked);
          return Promise.resolve({ ok: false as const, kind: "documents", message: blocked });
        }
        const content = takeContent(composer, sessionId);
        if (content === null) {
          return Promise.resolve({
            ok: false as const,
            kind: "empty",
            message: "Nothing to send.",
          });
        }
        contentTaken?.(sessionId, content.context);
        const bound = origin === "debug-api" || content.by === "debug-api" ? "debug-api" : "ui";
        return load(bound)
          .then((impl) =>
            afterSaving(sessionId, bound, () =>
              impl.sendContent(sessionId, mode, asVoice(sessionId, content)),
            ),
          )
          .then(
            (outcome) => {
              contentSettled?.(sessionId, content.context, outcome.ok);
              return outcome;
            },
            (error: unknown) => {
              contentSettled?.(sessionId, content.context, false);
              throw error;
            },
          );
      },
      /** Sends a button's user message separately; `trustedClick` preserves a native click's origin. */
      sendMessage(sessionId: string, text: string, trustedClick = false): Promise<SendOutcome> {
        const blocked = deps.keyGuard?.() ?? null;
        if (blocked !== null) return Promise.resolve({ ok: false, kind: "key", message: blocked });
        const entry = core.store.get().sessions[sessionId];
        if (
          composer.store.get().sending[sessionId] === true ||
          runningTurnId(entry?.state ?? null, entry?.session) !== null
        )
          return Promise.resolve({
            ok: false,
            kind: "busy",
            message: "The agent is working; try when it is done",
          });
        const content: SentContent = {
          literal: true,
          by: origin === "debug-api" ? "debug-api" : "user",
          text,
          context: [],
          images: [],
          restoreOnFailure: false,
        };
        // Reserve before lazy loading, so a second click cannot send the same request meanwhile.
        composer.setSending(sessionId, true);
        return load(origin === "ui" && trustedClick ? "window" : origin)
          .then((impl) =>
            afterSaving(sessionId, origin, () =>
              impl.sendContent(sessionId, "send", asVoice(sessionId, content)),
            ),
          )
          .finally(() => {
            composer.setSending(sessionId, false);
          });
      },
      /** Retry is another agent turn, so it follows the same readiness rule as Send. */
      retry(sessionId: string, instructions: string): Promise<SendOutcome> {
        const blocked = deps.keyGuard?.() ?? null;
        if (blocked !== null) return Promise.resolve({ ok: false, kind: "key", message: blocked });
        return load(origin).then((impl) => impl.retry(sessionId, instructions));
      },
      /**
       * Sends what the user said in voice chat as a voice turn: never through the text box (a draft the
       * user is typing stays), always literal (a spoken "/clear" or "!rm" is never a command), queued after a running
       * turn. In the new-chat view it starts the session first, like Send (`started` names it).
       */
      sendSpoken(sessionId: string, text: string): Promise<NewChatOutcome> {
        const blocked = deps.keyGuard?.() ?? null;
        if (blocked !== null) return Promise.resolve({ ok: false, kind: "key", message: blocked });
        const content: SentContent = {
          literal: true,
          by: origin === "debug-api" ? "debug-api" : "user",
          text,
          context: [],
          images: [],
          voice: true,
        };
        if (sessionId === NEW_CHAT) return sendNewChat(content);
        return load(origin).then((impl) =>
          afterSaving(sessionId, origin, () => impl.sendContent(sessionId, "queue", content)),
        );
      },
      /** Loads the implementation now (the composer view calls it when it mounts). */
      preload: (): void => {
        void load(origin);
      },
      /**
       * Puts quoted text into the composer of `sessionId` as a context chip. Always data: sent as a
       * resource block with the next message, never typed text, never a command. Marked `debug-api`
       * in the Debug API's set.
       */
      insertContext(sessionId: string, item: Omit<ContextItem, "id" | "by">): ContextItem {
        return composer.addContext(sessionId, {
          label:
            (item.ref === undefined ? item.label.trim().slice(0, 80) : item.label.trim()) ||
            "Context",
          source: item.source.trim().slice(0, 40) || "context",
          text: item.text,
          ...(item.ref === undefined ? {} : { ref: item.ref }),
          ...(origin === "debug-api" ? { by: "debug-api" as const } : {}),
        });
      },
      /** Closes the rewind dialog without rewinding. */
      closeRewind(): void {
        composer.setRewind(null);
      },
    };
  };

  // A deleted session's draft and chips go with it (removed from a complete listing, or deleted here).
  let known = new Set(Object.keys(core.store.get().sessions));
  core.store.subscribe(() => {
    const now = core.store.get().sessions;
    const gone = [...known].filter((id) => !(id in now) && !core.store.get().index.loading);
    known = new Set(Object.keys(now));
    if (gone.length > 0) {
      composer.forget(gone);
      for (const id of gone) {
        initialization.ready.delete(id);
        initialization.choices.delete(id);
      }
    }
  });

  /** What the window's own controls use. */
  const actions = facade("ui");
  /** What the `ui/composer` route uses: every call and every piece of content is the Debug API's. */
  const debugApi = facade("debug-api");
  /** Installs the check that can hold a message back (the documents controller). */
  const setSendGuard = (guard: (sessionId: string) => string | null): void => {
    sendGuard = guard;
  };
  const setContentTaken = (
    callback: (sessionId: string, context: readonly ContextItem[]) => void,
  ): void => {
    contentTaken = callback;
  };
  const setContentSettled = (
    callback: (sessionId: string, context: readonly ContextItem[], sent: boolean) => void,
  ): void => {
    contentSettled = callback;
  };
  /** Installs (or, with `null`, removes) how the new-chat view starts its session. */
  const setNewChatStarter = (starter: NewChatStarter | null): void => {
    newChatStarter = starter;
  };
  /** Installs (or, with `null`, removes) the step that runs before a message goes out. */
  const setBeforeSend = (hook: BeforeSend | null): void => {
    beforeSend = hook;
  };
  /** Installs what moves the New chat screen's open things to the session its first message starts. */
  const setNewChatHandOver = (handOver: NewChatHandOver | null): void => {
    newChatHandOver = handOver;
  };
  /** Installs the voice chat's test of whether a session's turns are voice turns. */
  const setVoiceTurns = (test: (sessionId: string) => boolean): void => {
    voiceTurns = test;
  };
  return {
    ...composer,
    core,
    actions,
    debugApi,
    setSendGuard,
    setContentTaken,
    setContentSettled,
    setNewChatStarter,
    setNewChatHandOver,
    setVoiceTurns,
    setBeforeSend,
  } as const;
}

/** The composer. `actions` is `null` without a core. */
export type Composer = ReturnType<typeof createComposer>;
