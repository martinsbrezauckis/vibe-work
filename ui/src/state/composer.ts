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

/** The asynchronous actions the facade forwards to the loaded implementation. */
const DEFERRED = [
  "stop",
  "retry",
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
  start(origin: "ui" | "debug-api"): Promise<string | null>;
}

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
      setVoiceTurns: (): void => undefined,
    } as const;
  }

  core.listen((event) => {
    if (event.type === "alert" && event.kind === "pluginReloadFailed" && event.sessionId !== null)
      composer.pluginReloadRefused(event.sessionId);
  });
  const chains = new Map<string, Promise<unknown>>();
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
  // Whether a session is in voice chat (its turns then go out as voice turns); set by the voice controller.
  let voiceTurns: ((sessionId: string) => boolean) | null = null;
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
      try {
        const impl = await load(bound);
        await impl.loadConfig();
        const started =
          newChatStarter === null
            ? ((await deps.sessions?.startIn(
                folder,
                bound === "debug-api" ? "debug-api" : undefined,
              )) ?? null)
            : await newChatStarter.start(bound);
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
        let picked = await impl.prepareSession(started, true, held);
        composer.resetNewChat();
        if (picked && held.mode !== null) {
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
            "Not sent yet: the access mode or model you chose was not set. Your message is in the box; send it when ready.";
          composer.note(started, message);
          return { ok: false, kind: "picks", message, started };
        }
        return { ...(await impl.sendContent(started, "send", content)), started };
      } finally {
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
          .then((impl) => impl.sendContent(sessionId, mode, asVoice(sessionId, content)))
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
          .then((impl) => impl.sendContent(sessionId, "send", asVoice(sessionId, content)))
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
        return load(origin).then((impl) => impl.sendContent(sessionId, "queue", content));
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
          label: item.label.trim().slice(0, 80) || "Context",
          source: item.source.trim().slice(0, 40) || "context",
          text: item.text,
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
    setVoiceTurns,
  } as const;
}

/** The composer. `actions` is `null` without a core. */
export type Composer = ReturnType<typeof createComposer>;
