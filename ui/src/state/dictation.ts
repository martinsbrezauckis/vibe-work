/**
 * Dictation: the microphone -> the core
 * (`speech` start/append/stop/cancel, which calls Voxtral with the key from the vault) -> the text at the
 * caret of the session's text box. The button state machine is adapted from ShellX's MicButton;
 * it moves through idle, recording, transcribing and error
 * states. Starting a recording follows this order:
 * 1. prepare the audio graph (no microphone yet; its rate is needed by the core),
 * 2. ask the core to start a stream,
 * 3. only then open the microphone; a refusal cancels the stream.
 * Depends on: coreStore, composer (drafts), voiceSettings, toastStore, features/voice/pcmCapture types.
 * Called by: createApp (via `createVoice`), the mic button, the shortcut, the `ui/voice` route.
 * Invariants:
 * - A start asked for while the previous dictation is still being made into text (phase `transcribing`, while
 *   the button says "stopping") is queued, not lost: it starts right after the text is in. A second ask cancels the
 *   queue; a failed stop or a cancel drops it. `queued` is in the state for the button and the route.
 * - The text is inserted as the user's own typing (typed-equivalent, never a context chip). A dictation
 *   started through the Debug API inserts with that API's provenance mark, so it is never a command
 *   until the user adopts it.
 * - Chunks are sent in order (one chain); `stop()` waits for them before asking for the text.
 * - One dictation at a time; every failure releases the microphone and the core stream.
 */
import type { CoreEvent, DictationMode, SpeechRequest, SpeechResult } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { PrepareCapture } from "../features/voice/pcmCapture";
import { micErrorText } from "../features/voice/pcmCapture";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { VoiceSettingsStore } from "./voiceSettings";

/** How long the microphone may take to open before the strip says it waits for the permission (an OS prompt). */
const PERMISSION_LINE_AFTER_MS = 500;

/** Where a dictation is. */
type DictationPhase = "idle" | "starting" | "recording" | "transcribing" | "error";

/** The dictation state the views and the route read. */
export interface DictationState {
  phase: DictationPhase;
  sessionId: string | null;
  streamId: string | null;
  mode: DictationMode | null;
  /** Who started it (`debug-api`: inserted with the external-client mark). */
  by: "ui" | "debug-api";
  /** Seconds of audio the core counted. */
  seconds: number;
  maxSeconds: number;
  /** Live text so far (realtime). */
  partial: string;
  /** A note to show (live text lost, the length limit, the fallback). */
  note: string | null;
  /** Why the last dictation failed (phase `error`). */
  error: string | null;
  /** The last text inserted. */
  lastText: string | null;
  /** A start is waiting for the text of the previous dictation: it begins once that text is in. */
  queued: boolean;
  /**
   * The microphone is being opened (`getUserMedia` pending). On a first use the OS may be asking the
   * user (macOS); the core's no-audio timeout does not run before the first audio, so this can last.
   */
  micPending: boolean;
  /** `micPending` for over half a second: the strip says it waits for the microphone permission. */
  permissionWait: boolean;
}

/** The state with nothing going on. */
export const DICTATION_IDLE: DictationState = {
  phase: "idle",
  sessionId: null,
  streamId: null,
  mode: null,
  by: "ui",
  seconds: 0,
  maxSeconds: 300,
  partial: "",
  note: null,
  error: null,
  lastText: null,
  queued: false,
  micPending: false,
  permissionWait: false,
};

/** Where the text box should put its caret after an insertion (`seq` changes each time). */
export type CaretPlacement = { sessionId: string; caret: number; seq: number } | null;

/** Base64 of bytes (chunked: `String.fromCharCode` takes a limited number of arguments). */
export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

/**
 * The draft after inserting `text` at `[start, end)` (the selection is replaced), with a space added where
 * the text would otherwise touch a word; `caret` is where the caret goes after it.
 */
export function insertAt(
  draft: string,
  start: number,
  end: number,
  text: string,
): { draft: string; caret: number } {
  const a = Math.max(0, Math.min(start, draft.length));
  const b = Math.max(a, Math.min(end, draft.length));
  const before = draft.slice(0, a);
  const after = draft.slice(b);
  const lead = before !== "" && !/\s$/.test(before) ? " " : "";
  const tail = after !== "" && !/^\s/.test(after) ? " " : "";
  const piece = `${lead}${text.trim()}${tail}`;
  return { draft: `${before}${piece}${after}`, caret: before.length + piece.length - tail.length };
}

/** Creates the dictation controller. */
export function createDictation(deps: {
  core: CoreController;
  composer: Composer;
  settings: VoiceSettingsStore;
  prepare: PrepareCapture;
  canUseKey?: (() => boolean) | undefined;
  /** Called when a dictation starts (narration stops: never narrate while dictating). */
  onStart: () => void;
  /** Why a dictation cannot start now (voice chat holds the microphone), or `null`. */
  blocked?: () => string | null;
}) {
  const { core, composer, settings } = deps;
  const store = createStore<DictationState>(DICTATION_IDLE);
  /** The caret per session as the text box last reported it by an event: the fallback only. */
  const carets = new Map<string, { start: number; end: number }>();
  /**
   * Live readers of the text box's own selection per session (the mounted text box registers one). The
   * reported caret above can be stale or missing (a click on the mic button takes the focus away, a
   * draft changed by code moves the caret without an event, a late event overwrites a newer value), so
   * the insert asks the text box itself.
   */
  const caretReaders = new Map<string, (draft: string) => { start: number; end: number } | null>();
  /** Where the view should put the caret after an insertion (`seq` changes each time). */
  const placed = createStore<CaretPlacement>(null);
  let capture: { stop(): void } | null = null;
  let chain: Promise<unknown> = Promise.resolve();
  /** A stop asked for while the dictation was still starting (a short hold): honoured once recording. */
  let stopWanted = false;
  const stopRequested = (): boolean => stopWanted;
  /** Bumped by every start and cancel: a start that was cancelled while it waited gives up. */
  let attempt = 0;
  /** The start waiting for the current stop to finish (`store.queued` is true exactly while this is set). */
  let queuedStart: { sessionId: string; as: "ui" | "debug-api" } | null = null;

  const once = async (request: SpeechRequest): Promise<SpeechResult> => {
    const result = await core.run({ type: "speech", request });
    if (result.type !== "speech") throw new Error("unexpected answer to a speech request");
    return result.result;
  };
  // Every append and stop is the window's (also for a dictation a Debug API request started), so no retry is
  // needed.
  const run = once;
  const fail = (message: string): void => {
    capture?.stop();
    capture = null;
    // A failed dictation drops the queued start: the error is what the user sees.
    queuedStart = null;
    store.set((s) => ({
      ...s,
      phase: "error",
      micPending: false,
      permissionWait: false,
      error: message,
      streamId: null,
      partial: "",
      queued: false,
    }));
  };
  const insert = (sessionId: string, text: string, by: "ui" | "debug-api"): void => {
    if (text.trim() === "") return;
    const draft = composer.store.get().drafts[sessionId] ?? "";
    // The text box's own selection wins; the reported one only when the box is not mounted or its text
    // differs from the draft (a render still pending, so its selection refers to other text).
    const caret = caretReaders.get(sessionId)?.(draft) ??
      carets.get(sessionId) ?? { start: draft.length, end: draft.length };
    const next = insertAt(draft, caret.start, caret.end, text);
    if (by === "debug-api") composer.setDraftFromDebugApi(sessionId, next.draft);
    else composer.setDraft(sessionId, next.draft);
    carets.set(sessionId, { start: next.caret, end: next.caret });
    placed.set((p) => ({ sessionId, caret: next.caret, seq: (p?.seq ?? 0) + 1 }));
  };

  const append = (streamId: string, pcm: Uint8Array): void => {
    chain = chain.then(async () => {
      if (store.get().streamId !== streamId) return;
      if (deps.canUseKey?.() === false) {
        cancel();
        return;
      }
      try {
        const r = await run({ op: "append", streamId, pcm: toBase64(pcm) });
        if (r.kind !== "appended") return;
        store.set((s) => (s.streamId === streamId ? { ...s, seconds: r.seconds } : s));
        if (r.atLimit && store.get().phase === "recording") {
          const max = store.get().maxSeconds;
          store.set((s) => ({
            ...s,
            note: `Dictation stops at ${String(Math.round(max / 60))} minutes.`,
          }));
          void stop();
        }
      } catch (thrown) {
        if (deps.canUseKey?.() === false) {
          cancel();
          return;
        }
        if (store.get().streamId === streamId) fail(toCallError(thrown).error.message);
      }
    });
  };

  /** Starts a dictation into `sessionId`'s text box. `as` = whose action it is. */
  const start = async (sessionId: string, as: "ui" | "debug-api" = "ui"): Promise<void> => {
    if (deps.canUseKey?.() === false) return;
    const phase = store.get().phase;
    if (phase !== "idle" && phase !== "error") return;
    const refused = deps.blocked?.() ?? null;
    if (refused !== null) {
      store.set((s) => ({ ...s, phase: "error", error: refused, sessionId }));
      return;
    }
    stopWanted = false;
    attempt += 1;
    const ticket = attempt;
    deps.onStart();
    store.set({
      ...DICTATION_IDLE,
      phase: "starting",
      sessionId,
      by: as,
      lastText: store.get().lastText,
    });
    let prepared;
    try {
      prepared = await deps.prepare();
    } catch (thrown) {
      if (deps.canUseKey?.() === false) {
        cancel();
        return;
      }
      if (ticket === attempt) fail(micErrorText(thrown));
      return;
    }
    if (ticket !== attempt || deps.canUseKey?.() === false) {
      prepared.discard();
      if (deps.canUseKey?.() === false) cancel();
      return;
    }
    let started: SpeechResult;
    try {
      started = await run({
        op: "start",
        sampleRate: prepared.sampleRate,
        language: settings.store.get().language,
      });
    } catch (thrown) {
      prepared.discard();
      if (deps.canUseKey?.() === false) {
        cancel();
        return;
      }
      if (ticket !== attempt) return;
      const error = toCallError(thrown).error;
      fail(
        error.code === "confirmationDeclined"
          ? "Dictation was not started: the request was declined."
          : error.message,
      );
      return;
    }
    if (started.kind !== "started") return;
    const streamId = started.streamId;
    if (ticket !== attempt || deps.canUseKey?.() === false) {
      prepared.discard();
      void run({ op: "cancel", streamId }).catch(() => undefined);
      if (deps.canUseKey?.() === false) cancel();
      return;
    }
    store.set((s) => ({
      ...s,
      streamId,
      mode: started.mode,
      maxSeconds: started.maxSeconds,
      micPending: true,
    }));
    // A quick open (the permission is known) never shows the permission line; a slow one (an OS prompt) does.
    const slow = setTimeout(() => {
      store.set((s) =>
        s.streamId === streamId && s.micPending ? { ...s, permissionWait: true } : s,
      );
    }, PERMISSION_LINE_AFTER_MS);
    try {
      capture = await prepared.open({
        deviceId: settings.store.get().micId,
        onChunk: (pcm) => {
          append(streamId, pcm);
        },
      });
    } catch (thrown) {
      void run({ op: "cancel", streamId }).catch(() => undefined);
      if (deps.canUseKey?.() === false) {
        cancel();
        return;
      }
      if (ticket === attempt) fail(micErrorText(thrown));
      return;
    } finally {
      clearTimeout(slow);
    }
    store.set((s) => ({ ...s, micPending: false, permissionWait: false }));
    if (ticket !== attempt || store.get().streamId !== streamId || deps.canUseKey?.() === false) {
      capture.stop();
      capture = null;
      if (deps.canUseKey?.() === false) cancel();
      return;
    }
    store.set((s) => ({ ...s, phase: "recording" }));
    // `stopWanted` may have been set by `stop` while this function awaited (a short hold).
    if (stopRequested()) await stop();
  };

  /** Stops the microphone and inserts the text (while starting: as soon as recording begins). */
  const stop = async (): Promise<string | null> => {
    const s = store.get();
    if (s.phase === "starting") {
      stopWanted = true;
      return null;
    }
    if (s.phase !== "recording" || s.streamId === null || s.sessionId === null) return null;
    if (deps.canUseKey?.() === false) {
      cancel();
      return null;
    }
    const { streamId, sessionId, by } = s;
    capture?.stop();
    capture = null;
    store.set((x) => ({ ...x, phase: "transcribing" }));
    await chain;
    if (store.get().streamId !== streamId) return null;
    if (deps.canUseKey?.() === false) {
      cancel();
      return null;
    }
    try {
      const r = await run({ op: "stop", streamId });
      if (deps.canUseKey?.() === false) {
        cancel();
        return null;
      }
      if (r.kind !== "transcript") {
        // Never left in "transcribing" (and never with a queued start) on an answer the protocol does not expect.
        fail(`Unexpected answer to stop: ${r.kind}`);
        return null;
      }
      insert(sessionId, r.text, by);
      // "Send after dictation" sends the message once the dictated text is in, as the same
      // origin that dictated (the Debug API's dictation sends as the Debug API).
      if (settings.store.get().autosend && r.text.trim() !== "") {
        const actions = by === "debug-api" ? composer.debugApi : composer.actions;
        void actions?.send(sessionId, "send");
      }
      const next = queuedStart;
      queuedStart = null;
      store.set((x) => ({
        ...DICTATION_IDLE,
        lastText: r.text,
        note: r.note ?? (r.text === "" ? "Nothing was heard." : x.note),
        maxSeconds: x.maxSeconds,
      }));
      // The press that came while the text was being made: dictation starts now.
      if (next !== null) void start(next.sessionId, next.as);
      return r.text;
    } catch (thrown) {
      if (deps.canUseKey?.() === false) {
        cancel();
        return null;
      }
      fail(toCallError(thrown).error.message);
      return null;
    }
  };

  /** Drops the dictation (nothing is inserted). */
  const cancel = (): void => {
    stopWanted = false;
    queuedStart = null;
    attempt += 1;
    const { streamId } = store.get();
    capture?.stop();
    capture = null;
    store.set((s) => ({ ...DICTATION_IDLE, lastText: s.lastText }));
    if (streamId !== null) void run({ op: "cancel", streamId }).catch(() => undefined);
  };

  /**
   * A press that asks for a dictation while the previous one is still being made into text (phase
   * `transcribing`): the first one queues a start for `sessionId` (it begins right after the text is in), the
   * next one cancels the queued start. Returns whether a start is queued afterwards; in any other phase it
   * does nothing and returns false (the caller starts or stops as usual).
   */
  const queueStart = (sessionId: string, as: "ui" | "debug-api" = "ui"): boolean => {
    if (deps.canUseKey?.() === false) return false;
    if (store.get().phase !== "transcribing") return false;
    queuedStart = queuedStart === null ? { sessionId, as } : null;
    const queued = queuedStart !== null;
    store.set((s) => ({ ...s, queued }));
    return queued;
  };

  core.listen((event: CoreEvent) => {
    if (event.type !== "dictation" || event.streamId !== store.get().streamId) return;
    if (deps.canUseKey?.() === false) {
      cancel();
      return;
    }
    const u = event.update;
    if (u.type === "partial") store.set((s) => ({ ...s, partial: u.text }));
    else if (u.type === "liveLost")
      store.set((s) => ({ ...s, note: "Live text stopped; the text comes when you stop." }));
    else fail(u.reason);
  });

  return {
    store,
    placed,
    start,
    stop,
    cancel,
    queueStart,
    /** The text box reports its selection, so the text lands where the user is typing. */
    setCaret(sessionId: string, start: number, end: number): void {
      carets.set(sessionId, { start, end });
    },
    /**
     * The mounted text box of `sessionId` registers a reader of its own selection, given the draft being
     * edited (`null` when it cannot answer: unmounted, or showing other text). Returns the function that removes it.
     */
    bindCaretReader(
      sessionId: string,
      read: (draft: string) => { start: number; end: number } | null,
    ): () => void {
      caretReaders.set(sessionId, read);
      return () => {
        if (caretReaders.get(sessionId) === read) caretReaders.delete(sessionId);
      };
    },
    /** Clears an error or note. */
    dismiss(): void {
      store.set((s) =>
        s.phase === "error" || s.note !== null
          ? { ...s, phase: s.phase === "error" ? "idle" : s.phase, error: null, note: null }
          : s,
      );
    },
    /** True while the microphone is on or a text is being made. */
    busy(): boolean {
      const p = store.get().phase;
      return p === "starting" || p === "recording" || p === "transcribing";
    },
  };
}
