/**
 * The composer's state: per session the draft text, the context chips and attached images,
 * sends in flight, the keys of a failed send (so pressing Enter again resends without a duplicate turn),
 * the skills for the `/` menu, the session's loops (recurring prompts) as last read, which menu is open, the queued
 * turn being edited and the rewind dialog.
 * Plain state and small mutations only; the actions that talk to the core are in `composerSend.ts`,
 * `composerSession.ts` and `composerRewind.ts`.
 * Depends on: createStore, the composer model (drafts, message types). Called by: createComposer
 * (`composer.ts`), the composer views (read only), tests.
 * Invariants:
 * - Context chips and images are separate from the text: nothing the app inserts ever becomes typed
 *   text ("sent context is never a command"). Text the app puts back into the box (a rewound
 *   message, which may come from another client such as the CLI or the Debug API) is marked `literal`:
 *   it is sent as a message, never run as a `!cmd` or `/command`, until the user edits it (security
 *   review of the draft).
 * - Provenance, not timing (the trust rule): text, chips and images a Debug API client
 *   wrote through `ui/composer` are marked `debug-api` (`draftBy`, `ContextItem.by`, `ImageItem.by`).
 *   A message holding any of it is sent with the Debug API origin whoever presses Enter (a label;
 *   the Debug API acts as the user, so its typed `!cmd` / `/cmd` runs like the user's; `takeContent` reports
 *   `by`); marks are sticky: the draft marks (`draftBy`, `literal`) end only when the box becomes empty or with
 *   `adopt` (the explicit "Use as my command" of a rewound message), a chip or image mark only when that item
 *   is removed. Every state change that drops or restores content keeps the
 *   marks with it (`clear`, `restore`, `forget`), so the state never drifts to "user" on its own.
 * - Drafts are stored on every change (when `persist`) WITH their provenance (`user` / `debug-api` /
 *   `literal`). The default reasoning preference is stored only on an explicit pick. Images and context
 *   stay in memory: they can be large and belong to one message. Unknown draft provenance loads as Debug API.
 * - A long paste is held aside (`pastes`, in memory) behind a marker in the draft; `takeContent` puts the
 *   full text back in place of each marker, so what is sent is the message as pasted. Only what the user
 *   can see is a command: a message that starts with a marker is sent as text (`literal`).
 */
import type { JsonValue } from "../protocol/core";
import {
  type DraftBy,
  type Drafts,
  loadDrafts,
  saveDrafts,
  withDraft,
  withoutDrafts,
} from "../features/composer/model/drafts";
import type { LoopRow } from "../features/composer/model/loops";
import type { ContextItem, ImageItem } from "../features/composer/model/message";
import { queuedBack } from "../features/composer/model/queuedBack";
import {
  type PasteItem,
  expandPastes,
  pasteMarker,
  pastesIn,
} from "../features/composer/model/pastes";
import { createStore } from "./createStore";
import { loadDefaultReasoning, saveDefaultReasoning } from "./defaultReasoning";

/** Keys of a send that failed in transport, reused when the same content is sent again. */
/** The plain note when Vibe refuses to load changed plugins into an open chat. */
export const PLUGIN_RELOAD_REFUSED =
  "This chat could not refresh its plugins and still has its previous plugins.";

/** Both the definitive refusal and the uncertain-reply message describe a held plugin refresh. */
export const isPluginReloadFailure = (message: string): boolean =>
  message.includes(PLUGIN_RELOAD_REFUSED) ||
  message.includes("This chat could not finish refreshing its plugins");

export interface ResendKeys {
  /** The content they belong to (`contentKey`). */
  content: string;
  idempotencyKey: string;
  clientUserMessageId: string;
}

/** A skill as the `/` menu lists it. */
export interface SkillEntry {
  name: string;
  description: string;
}

/** The rewind dialog. */
export interface RewindDialog {
  sessionId: string;
  entryId: string;
  /** The message being rewound to (shown in the dialog). */
  text: string;
  loading: boolean;
  /** Files Vibe will restore (`session/rewind/read`). */
  paths: readonly string[];
  hasFileChanges: boolean;
  error: string | null;
  busy: boolean;
}

/** The composer's state. */
export interface ComposerState {
  drafts: Drafts;
  context: Readonly<Record<string, readonly ContextItem[]>>;
  images: Readonly<Record<string, readonly ImageItem[]>>;
  /** Long pastes held behind a marker in the draft, until the message is sent or cleared. */
  pastes: Readonly<Record<string, readonly PasteItem[]>>;
  /** Sessions with a send in flight. */
  sending: Readonly<Record<string, true>>;
  /** Manual context compaction in progress, cancellable through Stop. */
  compacting: Readonly<Record<string, true>>;
  /** A short message under the composer per session (a refused send, a stale steer). */
  notes: Readonly<Record<string, string>>;
  /** Chats whose plugin reload failed; their note offers a new chat. */
  pluginReloadFailed: Readonly<Record<string, true>>;
  resend: Readonly<Record<string, ResendKeys>>;
  skills: Readonly<Record<string, readonly SkillEntry[]>>;
  /**
   * Each session's loops as `loops/list` last answered (absent: not read yet). The session row's mark and the Loops
   * list read it; only `composerLoops.ts` writes it.
   */
  loops: Readonly<Record<string, readonly LoopRow[]>>;
  /** Sessions whose Loops list is expanded (collapsed is the default: one line). */
  loopsOpen: Readonly<Record<string, true>>;
  /** Models from `config/read`, for sessions whose runtime is not known yet. */
  config: JsonValue | null;
  /** New-chat preference; null follows the model. Unsupported saved levels stay saved. */
  defaultReasoning: string | null;
  menu: { sessionId: string; kind: "access" | "model" } | null;
  /** The highlighted row of the `/` menu. */
  slashIndex: number;
  editing: { sessionId: string; queueItemId: string; text: string } | null;
  rewind: RewindDialog | null;
  /** Sessions whose draft the app put back (a rewound message): sent as text until the user edits it. */
  literal: Readonly<Record<string, true>>;
  /** Sessions whose draft text a Debug API client wrote (until the user edits it). */
  draftBy: Readonly<Record<string, "debug-api">>;
  /** The new-chat view: picks held until the first send, and whether Send asked for a folder. */
  newChat: NewChatHeld;
}

/**
 * What the new-chat view holds before its session exists: the access mode and model the user picked
 * (`null`: Vibe's default), applied to the new session before its first turn; `askFolder` lights up the folder
 * chip after a Send without a folder.
 */
export interface NewChatHeld {
  mode: string | null;
  /** Consent from the latest native access pick; never granted by synthetic or Debug API picks. */
  trustedAccessPick: boolean;
  alias: string | null;
  effort: string | null;
  modelName?: string;
  modelDisplayName?: string;
  /** Who held a pick: `debug-api` once any held pick came from a Debug API client (provenance), `null`: none. */
  by: "ui" | "debug-api" | null;
  askFolder: boolean;
}

const NO_PICKS: NewChatHeld = {
  mode: null,
  trustedAccessPick: false,
  alias: null,
  effort: null,
  by: null,
  askFolder: false,
};

const EMPTY: ComposerState = {
  drafts: {},
  context: {},
  images: {},
  pastes: {},
  sending: {},
  compacting: {},
  notes: {},
  pluginReloadFailed: {},
  resend: {},
  skills: {},
  loops: {},
  loopsOpen: {},
  config: null,
  defaultReasoning: null,
  menu: null,
  slashIndex: 0,
  editing: null,
  rewind: null,
  literal: {},
  draftBy: {},
  newChat: NO_PICKS,
};

/** One message as it was in the composer when the user pressed the key. */
export interface SentContent {
  /** Recovered drafts need explicit adoption; separate button commands keep their own intent. */
  literal: boolean;
  /** Who wrote any of it: `debug-api` when a Debug API client did (the send then uses its origin). */
  by: "user" | "debug-api";
  text: string;
  context: readonly ContextItem[];
  images: readonly ImageItem[];
  /** Separate button requests leave the draft untouched, including after a failed send. */
  restoreOnFailure?: false;
  /**
   * A voice chat turn: it goes out as a queued turn whose context entry carries the voice instruction
   * (never inside the text), and its display content carries the "voice" marker. A steer stays a plain steer.
   */
  voice?: boolean;
}

/**
 * Takes the composer content of `sessionId` and empties the composer, synchronously (so a key press
 * sends exactly what was in the box at that moment, even while the action code is still loading), or
 * returns `null` when there is nothing to send.
 */
export function takeContent(composer: ComposerStore, sessionId: string): SentContent | null {
  const s = composer.store.get();
  const context = s.context[sessionId] ?? [];
  const images = s.images[sessionId] ?? [];
  const draft = s.drafts[sessionId] ?? "";
  const pastes = pastesIn(draft, s.pastes[sessionId] ?? []);
  const text = expandPastes(draft, pastes);
  // The text a paste brings in is never the start of a `!cmd` or `/command` the user did not see.
  const hidden = pastes.some((item) => draft.trimStart().startsWith(pasteMarker(item)));
  const fromApi =
    (s.draftBy[sessionId] === "debug-api" && text !== "") ||
    context.some((c) => c.by === "debug-api") ||
    images.some((i) => i.by === "debug-api");
  const content: SentContent = {
    // Debug API text is the user's (its `!cmd` runs); only a rewound message is literal.
    literal: s.literal[sessionId] === true || hidden,
    by: fromApi ? "debug-api" : "user",
    text,
    context,
    images,
  };
  if (content.text.trim() === "" && content.context.length === 0 && content.images.length === 0) {
    return null;
  }
  composer.take(sessionId);
  return content;
}

/** A key that identifies the content of one message (text + chips + images). */
export function contentKey(
  text: string,
  context: readonly ContextItem[],
  images: readonly ImageItem[],
): string {
  return JSON.stringify([
    text,
    context.map((c) => [c.label, c.source, c.text]),
    images.map((i) => i.data),
  ]);
}

function setIn<T>(
  map: Readonly<Record<string, T>>,
  id: string,
  value: T | undefined,
): Record<string, T> {
  if (value !== undefined) return { ...map, [id]: value };
  if (!(id in map)) return map;
  return Object.fromEntries(Object.entries(map).filter(([key]) => key !== id));
}

/** Creates the store; `persist` keeps drafts and explicit default reasoning picks in app storage. */
export function createComposerStore(persist: boolean) {
  const stored = persist ? loadDrafts() : { drafts: {}, by: {} };
  const marks = (by: string) =>
    Object.fromEntries(
      Object.entries(stored.by)
        .filter(([, who]) => who === by)
        .map(([id]) => [id, by]),
    );
  const store = createStore<ComposerState>({
    ...EMPTY,
    defaultReasoning: persist ? loadDefaultReasoning() : null,
    drafts: stored.drafts,
    draftBy: marks("debug-api") as Record<string, "debug-api">,
    literal: Object.fromEntries(Object.keys(marks("literal")).map((id) => [id, true as const])),
  });
  let saved: Pick<ComposerState, "drafts" | "draftBy" | "literal"> = store.get();
  if (persist) {
    store.subscribe(() => {
      const s = store.get();
      if (s.drafts === saved.drafts && s.draftBy === saved.draftBy && s.literal === saved.literal)
        return;
      saved = s;
      const by: Record<string, DraftBy> = {};
      for (const id of Object.keys(s.literal)) by[id] = "literal";
      for (const id of Object.keys(s.draftBy)) by[id] = "debug-api";
      saveDrafts(s.drafts, by);
    });
  }
  let counter = 0;
  let prepareContext:
    | ((sessionId: string, context: readonly ContextItem[]) => Promise<readonly ContextItem[]>)
    | null = null;
  const taking = new Set<string>();
  // Item ids outlive a pause and clearing/sending the draft: late partial events must remain harmless.
  const returnedQueueItems = new Map<string, Set<string>>();
  const nextId = (prefix: string) => {
    counter += 1;
    return `${prefix}-${String(Date.now())}-${String(counter)}`;
  };
  const clear = (sessionId: string): void => {
    store.set((s) => ({
      ...s,
      drafts: withDraft(s.drafts, sessionId, ""),
      context: setIn(s.context, sessionId, undefined),
      images: setIn(s.images, sessionId, undefined),
      pastes: setIn(s.pastes, sessionId, undefined),
      literal: setIn(s.literal, sessionId, undefined),
      draftBy: setIn(s.draftBy, sessionId, undefined),
    }));
  };

  return {
    store,
    /** Documents install the save step for attachments held before a chat exists. */
    setPrepareContext(prepare: typeof prepareContext): void {
      prepareContext = prepare;
    },
    /** Saves staged attachments in the final session folder before their paths reach Vibe. */
    prepareContext(
      sessionId: string,
      context: readonly ContextItem[],
    ): Promise<readonly ContextItem[]> {
      return prepareContext?.(sessionId, context) ?? Promise.resolve(context);
    },
    /** True only while a send takes content out of the composer. */
    isTaking: (sessionId: string): boolean => taking.has(sessionId),
    take(sessionId: string): void {
      taking.add(sessionId);
      try {
        clear(sessionId);
      } finally {
        taking.delete(sessionId);
      }
    },
    /** Saves the user's explicit new-chat reasoning preference, without changing existing chats. */
    setDefaultReasoning(level: string | null): void {
      if (persist) saveDefaultReasoning(level);
      store.set((s) => ({ ...s, defaultReasoning: level }));
    },
    /** A fresh local id for a chip or image. */
    nextId,
    /**
     * The user's own typing. Marks (`draftBy`, `literal`) are sticky: an edit keeps them (one keystroke
     * must not launder Debug API or put-back text into a command); they
     * end only when the box becomes empty, or with `adopt` (the explicit "Use as my command").
     */
    setDraft(sessionId: string, text: string): void {
      store.set((s) => {
        const drafts = withDraft(s.drafts, sessionId, text);
        const emptied = text === "";
        const literal = emptied ? setIn(s.literal, sessionId, undefined) : s.literal;
        const draftBy = emptied ? setIn(s.draftBy, sessionId, undefined) : s.draftBy;
        return drafts === s.drafts && s.slashIndex === 0
          ? s
          : { ...s, drafts, literal, draftBy, slashIndex: 0 };
      });
    },
    /** The user's explicit "Use as my command": the draft becomes their own text (origin ui). */
    adopt(sessionId: string): void {
      store.set((s) => ({
        ...s,
        literal: setIn(s.literal, sessionId, undefined),
        draftBy: setIn(s.draftBy, sessionId, undefined),
      }));
    },
    /** Text a Debug API client wrote (the `ui/composer` `type` action): marked until the user edits it. */
    setDraftFromDebugApi(sessionId: string, text: string): void {
      store.set((s) => ({
        ...s,
        drafts: withDraft(s.drafts, sessionId, text),
        literal: setIn(s.literal, sessionId, undefined),
        draftBy: setIn(s.draftBy, sessionId, text === "" ? undefined : "debug-api"),
        slashIndex: 0,
      }));
    },
    /** Text the app puts back (a rewound message), when the box is empty: sent as text only. */
    putBack(sessionId: string, text: string): boolean {
      if ((store.get().drafts[sessionId] ?? "") !== "" || text === "") return false;
      store.set((s) => ({
        ...s,
        drafts: withDraft(s.drafts, sessionId, text),
        literal: setIn(s.literal, sessionId, true),
      }));
      return true;
    },
    /**
     * Puts queued messages the core took back (a stopped or failed turn) into the composer: an empty draft
     * becomes their text (literal, like a rewound message), a draft the user has keeps its text and gets theirs
     * after a blank line; their images and context chips are added to the ones present.
     */
    takeBackQueued(sessionId: string, items: readonly JsonValue[]): boolean {
      const seen = returnedQueueItems.get(sessionId) ?? new Set<string>();
      const fresh = items.filter((item) => {
        const id =
          item !== null && typeof item === "object" && !Array.isArray(item) ? item["id"] : null;
        if (typeof id !== "string" || seen.has(id)) return false;
        seen.add(id);
        return true;
      });
      if (fresh.length === 0) return false;
      returnedQueueItems.set(sessionId, seen);
      const back = queuedBack(fresh);
      store.set((s) => {
        const draft = s.drafts[sessionId] ?? "";
        const text =
          back.text === "" ? draft : draft === "" ? back.text : `${draft}\n\n${back.text}`;
        const images = [
          ...(s.images[sessionId] ?? []),
          ...back.images.map((image) => ({ ...image, id: nextId("img") })),
        ];
        const context = [
          ...(s.context[sessionId] ?? []),
          ...back.context.map((item) => ({ ...item, id: nextId("ctx") })),
        ];
        return {
          ...s,
          drafts: withDraft(s.drafts, sessionId, text),
          literal: draft === "" && back.text !== "" ? setIn(s.literal, sessionId, true) : s.literal,
          images: setIn(s.images, sessionId, images.length > 0 ? images : undefined),
          context: setIn(s.context, sessionId, context.length > 0 ? context : undefined),
        };
      });
      return true;
    },
    /** Adds a context chip (the text is data, shown as a chip, never put in the draft). */
    addContext(sessionId: string, item: Omit<ContextItem, "id">): ContextItem {
      const added: ContextItem = { ...item, id: nextId("ctx") };
      store.set((s) => ({
        ...s,
        context: setIn(s.context, sessionId, [...(s.context[sessionId] ?? []), added]),
      }));
      return added;
    },
    removeContext(sessionId: string, id: string): void {
      store.set((s) => {
        const left = (s.context[sessionId] ?? []).filter((c) => c.id !== id);
        return { ...s, context: setIn(s.context, sessionId, left.length > 0 ? left : undefined) };
      });
    },
    /** Holds a long paste aside; the caller puts its marker in the draft. */
    addPaste(sessionId: string, item: PasteItem): void {
      store.set((s) => ({
        ...s,
        pastes: setIn(s.pastes, sessionId, [...(s.pastes[sessionId] ?? []), item]),
      }));
    },
    addImage(sessionId: string, image: ImageItem): void {
      store.set((s) => ({
        ...s,
        images: setIn(s.images, sessionId, [...(s.images[sessionId] ?? []), image]),
      }));
    },
    removeImage(sessionId: string, id: string): void {
      store.set((s) => {
        const left = (s.images[sessionId] ?? []).filter((i) => i.id !== id);
        return { ...s, images: setIn(s.images, sessionId, left.length > 0 ? left : undefined) };
      });
    },
    /** Empties the composer of `sessionId` (after a send left it). */
    clear,
    /**
     * Puts a message back after a failed send, unless the user already typed something new (then the
     * new text wins). `keepBoth` retains a held plugin send alongside newer typing, including attachments and
     * provenance; a combined draft stays literal until explicitly adopted.
     */
    restore(
      sessionId: string,
      text: string,
      context: readonly ContextItem[],
      images: readonly ImageItem[],
      literal = false,
      by: "user" | "debug-api" = "user",
      keepBoth = false,
    ): boolean {
      const s = store.get();
      const busy =
        (s.drafts[sessionId] ?? "") !== "" ||
        (s.context[sessionId] ?? []).length > 0 ||
        (s.images[sessionId] ?? []).length > 0;
      if (busy && !keepBoth) return false;
      if (busy) {
        text = [text, s.drafts[sessionId] ?? ""].filter(Boolean).join("\n\n");
        context = [...context, ...(s.context[sessionId] ?? [])];
        images = [...images, ...(s.images[sessionId] ?? [])];
        // Combined drafts must stay text until explicitly adopted as a command.
        literal = true;
        if (s.draftBy[sessionId] === "debug-api") by = "debug-api";
      }
      store.set((prev) => ({
        ...prev,
        drafts: withDraft(prev.drafts, sessionId, text),
        context: setIn(prev.context, sessionId, context.length > 0 ? [...context] : undefined),
        images: setIn(prev.images, sessionId, images.length > 0 ? [...images] : undefined),
        literal: setIn(prev.literal, sessionId, literal ? true : undefined),
        draftBy: setIn(
          prev.draftBy,
          sessionId,
          by === "debug-api" && text !== "" ? "debug-api" : undefined,
        ),
      }));
      return true;
    },
    setSending(sessionId: string, on: boolean): void {
      store.set((s) => ({ ...s, sending: setIn(s.sending, sessionId, on ? true : undefined) }));
    },
    /** Tracks manual compaction independently of the turn stream. */
    setCompacting(sessionId: string, on: boolean): void {
      store.set((s) => ({
        ...s,
        compacting: setIn(s.compacting, sessionId, on ? true : undefined),
      }));
    },
    note(sessionId: string, text: string | null): void {
      store.set((s) => ({
        ...s,
        notes: setIn(s.notes, sessionId, text ?? undefined),
        pluginReloadFailed: setIn(
          s.pluginReloadFailed,
          sessionId,
          text === null
            ? undefined
            : isPluginReloadFailure(text)
              ? true
              : s.pluginReloadFailed[sessionId],
        ),
      }));
    },
    // Vibe's own reason ("cannot re-pin ... non-quiescent") is internal wording: it is logged, not shown.
    pluginReloadRefused(sessionId: string, message = PLUGIN_RELOAD_REFUSED): void {
      store.set((s) => ({
        ...s,
        pluginReloadFailed: setIn(s.pluginReloadFailed, sessionId, true),
        notes: setIn(s.notes, sessionId, message),
      }));
    },
    pluginReloaded(sessionId: string): void {
      store.set((s) => ({
        ...s,
        pluginReloadFailed: setIn(s.pluginReloadFailed, sessionId, undefined),
      }));
    },
    setResend(sessionId: string, keys: ResendKeys | null): void {
      store.set((s) => ({ ...s, resend: setIn(s.resend, sessionId, keys ?? undefined) }));
    },
    setSkills(sessionId: string, skills: readonly SkillEntry[]): void {
      store.set((s) => ({ ...s, skills: setIn(s.skills, sessionId, skills) }));
    },
    /** Keeps the loops `loops/list` answered for a session. */
    setLoops(sessionId: string, loops: readonly LoopRow[]): void {
      store.set((s) => ({ ...s, loops: setIn(s.loops, sessionId, loops) }));
    },
    /** Expands or collapses a session's Loops list. */
    setLoopsOpen(sessionId: string, open: boolean): void {
      store.set((s) => {
        if ((s.loopsOpen[sessionId] === true) === open) return s;
        return { ...s, loopsOpen: setIn(s.loopsOpen, sessionId, open ? true : undefined) };
      });
    },
    setConfig(config: JsonValue): void {
      store.set((s) => ({ ...s, config }));
    },
    setMenu(menu: ComposerState["menu"]): void {
      store.set((s) => (s.menu === menu ? s : { ...s, menu }));
    },
    setSlashIndex(index: number): void {
      store.set((s) => (s.slashIndex === index ? s : { ...s, slashIndex: index }));
    },
    setEditing(editing: ComposerState["editing"]): void {
      store.set((s) => ({ ...s, editing }));
    },
    /** Keeps an edit whose queued message has gone, alongside any message already being typed. */
    restoreQueueEdit(sessionId: string, text: string): void {
      store.set((s) => {
        const draft = s.drafts[sessionId] ?? "";
        return {
          ...s,
          drafts: withDraft(s.drafts, sessionId, draft === "" ? text : `${draft}\n\n${text}`),
          literal: setIn(s.literal, sessionId, true),
          editing: null,
        };
      });
    },
    setRewind(rewind: RewindDialog | null): void {
      store.set((s) => ({ ...s, rewind }));
    },
    /** Holds a pick for the new-chat view's first send, or sets `askFolder`. */
    holdNewChat(patch: Partial<NewChatHeld>): void {
      store.set((s) => ({ ...s, newChat: { ...s.newChat, ...patch } }));
    },
    /** Forgets the new-chat view's held picks (after its session started). */
    resetNewChat(): void {
      store.set((s) => (s.newChat === NO_PICKS ? s : { ...s, newChat: NO_PICKS }));
    },
    /** Drops everything kept for sessions that were deleted. */
    forget(ids: readonly string[]): void {
      for (const id of ids) returnedQueueItems.delete(id);
      store.set((s) => {
        const drafts = withoutDrafts(s.drafts, ids);
        let {
          context,
          images,
          pastes,
          resend,
          skills,
          loops,
          loopsOpen,
          notes,
          literal,
          draftBy,
          compacting,
          pluginReloadFailed,
        } = s;
        for (const id of ids) {
          literal = setIn(literal, id, undefined);
          draftBy = setIn(draftBy, id, undefined);
          context = setIn(context, id, undefined);
          images = setIn(images, id, undefined);
          pastes = setIn(pastes, id, undefined);
          resend = setIn(resend, id, undefined);
          skills = setIn(skills, id, undefined);
          loops = setIn(loops, id, undefined);
          loopsOpen = setIn(loopsOpen, id, undefined);
          notes = setIn(notes, id, undefined);
          compacting = setIn(compacting, id, undefined);
          pluginReloadFailed = setIn(pluginReloadFailed, id, undefined);
        }
        return {
          ...s,
          drafts,
          context,
          images,
          pastes,
          resend,
          skills,
          loops,
          loopsOpen,
          notes,
          literal,
          draftBy,
          compacting,
          pluginReloadFailed,
        };
      });
    },
  };
}

/** The store and its mutations. */
export type ComposerStore = ReturnType<typeof createComposerStore>;
