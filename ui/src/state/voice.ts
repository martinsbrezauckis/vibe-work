/**
 * Thin eager voice facade: owns the reactive voice stores synchronously while deferring
 * `state/voiceImpl` (and every `features/voice` capture/audio/speech module) until first voice use.
 * Depends on: createStore, voiceSettings, voiceTypes, accessibility. Called by: createApp, UI tests.
 * Invariants: never statically imports `features/voice/*` runtime code; first use immediately shows
 * busy state (never a dead click); chunk-load errors clear busy state and offer Retry without
 * blocking the composer.
 */
import type { Ask, CoreEvent, JsonValue } from "../protocol/core";
import type { Player } from "../features/voice/audioPlayer";
import type { PrepareCapture } from "../features/voice/pcmCapture";
import { obj, str } from "../features/transcript/model/entries";
import { createAccessibility, type Accessibility } from "./accessibility";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import { createVoiceSettings } from "./voiceSettings";
import {
  type CaretPlacement,
  DICTATION_IDLE,
  type DictationState,
  NARRATION_IDLE,
  type NarrationState,
  VOICE_CHAT_OFF,
  VOICE_CHUNK_IDLE,
  type VoiceChatState,
  type VoiceChunkLoadState,
  type VoicePromptState,
  type VoiceQuestionState,
} from "./voiceTypes";
import type { VoiceImpl } from "./voiceImpl";
import { CHUNK_HELP, chunkFailureReason } from "../components/chunkError";

type VoiceImplModule = typeof import("./voiceImpl");
type ReadAloudEvent = Extract<CoreEvent, { type: "readAloud" }>;
type CaretReader = (draft: string) => { start: number; end: number } | null;

let preloadedImpl: VoiceImplModule | null = null;

/** Preloads the lazy voice runtime chunk (used by deterministic test rigs). */
export async function preloadVoiceImpl(): Promise<void> {
  preloadedImpl ??= await import("./voiceImpl");
}

/** Clears the cached voice runtime module (for lazy-loading and failure recovery tests). */
export function resetVoiceImplForTest(): void {
  preloadedImpl = null;
}

function readableChunkError(error: unknown): string {
  const reason = chunkFailureReason(error);
  return reason === "" ? CHUNK_HELP : reason;
}

/** Creates the lazy voice controller facade. */
export function createVoice(deps: {
  core: CoreController;
  composer: Composer;
  shell: Shell;
  toasts: Toasts;
  persist: boolean;
  accessibility?: Accessibility;
  prepare?: PrepareCapture;
  player?: Player;
  /** Live key readiness; omitted for development without a key service. */
  canUseKey?: () => boolean;
  /** Sends a card's answer as the card's own buttons do (the transcript's answer path). */
  answer?: (sessionId: string, callbackId: string, output: JsonValue) => Promise<boolean>;
  /** Optional loader override for chunk-load / failure-recovery tests. */
  loadImpl?: () => Promise<VoiceImplModule>;
}) {
  const settings = createVoiceSettings(deps.persist);
  const accessibility = deps.accessibility ?? createAccessibility(deps.persist);
  const dictationStore = createStore<DictationState>(DICTATION_IDLE);
  const placedStore = createStore<CaretPlacement>(null);
  const narrationStore = createStore<NarrationState>(NARRATION_IDLE);
  const voiceChatStore = createStore<VoiceChatState>(VOICE_CHAT_OFF);
  const promptStore = createStore<VoicePromptState | null>(null);
  const questionStore = createStore<VoiceQuestionState | null>(null);
  const chunkLoadStore = createStore<VoiceChunkLoadState>(VOICE_CHUNK_IDLE);

  const bufferedCarets = new Map<string, { start: number; end: number }>();
  const bufferedReaders = new Map<string, CaretReader>();
  const realReaderUnbinds = new WeakMap<CaretReader, () => void>();
  const pendingReadAlouds: ReadAloudEvent[] = [];
  const pendingNarrationTurns: { sessionId: string; turnId: string }[] = [];
  const pendingPromptAsks: Ask[] = [];

  let impl: VoiceImpl | null = null;
  let inFlightLoad: Promise<VoiceImpl | null> | null = null;
  let failedRetry: (() => Promise<void>) | null = null;
  let promptWatchActive = false;
  let realPromptUnwatch: (() => void) | null = null;
  let facadePromptStop: (() => void) | null = null;

  let dictationTicket = 0;
  let pendingDictation: {
    ticket: number;
    sessionId: string;
    as: "ui" | "debug-api";
    stopWanted: boolean;
  } | null = null;
  const readPendingDictation = () => pendingDictation;

  let voiceChatTicket = 0;
  let pendingVoiceChat: { ticket: number; sessionId: string } | null = null;

  const dictationBusy = (): boolean => {
    if (impl !== null) return impl.dictation.busy();
    const p = dictationStore.get().phase;
    return p === "starting" || p === "recording" || p === "transcribing";
  };
  let readDictationBusy = dictationBusy;

  // A session in voice chat sends voice turns even before voiceImpl has been loaded.
  deps.composer.setVoiceTurns((id) => {
    const s = voiceChatStore.get();
    return s.on && s.owner === id;
  });

  const initImpl = (mod: VoiceImplModule): VoiceImpl => {
    if (impl !== null) return impl;
    const created = mod.createVoiceImpl(
      {
        core: deps.core,
        composer: deps.composer,
        shell: deps.shell,
        toasts: deps.toasts,
        settings,
        accessibility,
        prepare: deps.prepare,
        player: deps.player,
        canUseKey: deps.canUseKey,
        answer: deps.answer,
        isDictating: () => readDictationBusy(),
      },
      {
        dictationStore,
        placedStore,
        narrationStore,
        voiceChatStore,
        promptStore,
        questionStore,
      },
    );
    for (const [sessionId, caret] of bufferedCarets) {
      created.dictation.setCaret(sessionId, caret.start, caret.end);
    }
    bufferedCarets.clear();
    for (const [sessionId, read] of bufferedReaders) {
      realReaderUnbinds.set(read, created.dictation.bindCaretReader(sessionId, read));
    }
    bufferedReaders.clear();

    if (facadePromptStop !== null) {
      facadePromptStop();
      facadePromptStop = null;
    }
    if (promptWatchActive) {
      realPromptUnwatch = created.prompts.watch();
      for (const ask of pendingPromptAsks.splice(0)) {
        void created.prompts.arrived(ask);
      }
    } else {
      pendingPromptAsks.length = 0;
    }

    for (const ev of pendingReadAlouds.splice(0)) {
      created.narration.handleReadAloud(ev);
    }
    for (const item of pendingNarrationTurns.splice(0)) {
      created.narration.enqueue(item);
    }

    impl = created;
    return created;
  };

  if (preloadedImpl !== null && deps.loadImpl === undefined) {
    impl = initImpl(preloadedImpl);
  }

  const rejectPendingReadAlouds = () => {
    for (const ev of pendingReadAlouds.splice(0)) {
      void deps.core
        .run({
          type: "speech",
          request: { op: "readingReady", requestId: ev.requestId, available: false },
        })
        .catch(() => undefined);
    }
  };

  const loadRuntime = async (
    actionLabel: string,
    retry?: () => Promise<void>,
  ): Promise<VoiceImpl | null> => {
    if (impl !== null) return impl;
    if (retry !== undefined) failedRetry = retry;
    if (inFlightLoad !== null) return inFlightLoad;
    chunkLoadStore.set({ phase: "loading", error: null, actionLabel });
    const loader = deps.loadImpl ?? (() => import("./voiceImpl"));
    inFlightLoad = loader()
      .then((mod) => {
        if (deps.loadImpl === undefined) preloadedImpl = mod;
        const created = initImpl(mod);
        chunkLoadStore.set(VOICE_CHUNK_IDLE);
        failedRetry = null;
        return created;
      })
      .catch((thrown: unknown) => {
        chunkLoadStore.set({
          phase: "error",
          error: readableChunkError(thrown),
          actionLabel,
        });
        pendingNarrationTurns.length = 0;
        rejectPendingReadAlouds();
        if (narrationStore.get().phase === "summarizing" && narrationStore.get().current === null) {
          narrationStore.set((s) => ({ ...s, phase: "idle" }));
        }
        return null;
      })
      .finally(() => {
        inFlightLoad = null;
      });
    return inFlightLoad;
  };

  // Eagerly watch lightweight core/setting triggers while voiceImpl is unloaded.
  const stopCoreWatch = deps.core.listen((event: CoreEvent) => {
    if (impl !== null) return;
    if (event.type === "readAloud") {
      if (narrationStore.get().phase === "idle") {
        narrationStore.set((s) => ({ ...s, phase: "summarizing", note: null }));
      }
      pendingReadAlouds.push(event);
      void loadRuntime("Read aloud", async () => {
        if (narrationStore.get().phase === "idle") {
          narrationStore.set((s) => ({ ...s, phase: "summarizing", note: null }));
        }
        pendingReadAlouds.push(event);
        await loadRuntime("Read aloud");
      });
      return;
    }
    if (
      event.type === "change" &&
      event.change.kind === "turnUpserted" &&
      settings.store.get().narration &&
      event.change.sessionId === deps.shell.store.get().selected
    ) {
      const turn = obj(event.change.turn);
      const turnId = str(turn?.["id"] ?? null);
      if (turn?.["status"] === "completed" && turnId !== null) {
        if (narrationStore.get().phase === "idle") {
          narrationStore.set((s) => ({ ...s, phase: "summarizing", note: null }));
        }
        pendingNarrationTurns.push({ sessionId: event.change.sessionId, turnId });
        void loadRuntime("narration");
      }
    }
  });
  if (impl === null && settings.store.get().narration) {
    void loadRuntime("narration");
  }
  const stopSettingsWatch = settings.store.subscribe(() => {
    if (impl === null && settings.store.get().narration) {
      void loadRuntime("narration");
    }
  });

  const startDictation = async (
    sessionId: string,
    as: "ui" | "debug-api" = "ui",
  ): Promise<void> => {
    if (impl !== null) return impl.dictation.start(sessionId, as);
    if (deps.canUseKey?.() === false) return;
    const phase = dictationStore.get().phase;
    if (phase !== "idle" && phase !== "error") return;
    if (voiceChatStore.get().on) {
      dictationStore.set((s) => ({
        ...s,
        phase: "error",
        error: "Voice chat is on: end it first, dictation and voice chat share the microphone.",
        sessionId,
      }));
      return;
    }
    dictationTicket += 1;
    const ticket = dictationTicket;
    pendingDictation = { ticket, sessionId, as, stopWanted: false };
    dictationStore.set({
      ...DICTATION_IDLE,
      phase: "starting",
      sessionId,
      by: as,
      lastText: dictationStore.get().lastText,
    });
    const ready = await loadRuntime("dictation", () => startDictation(sessionId, as));
    const current = readPendingDictation();
    if (ready === null) {
      if (current?.ticket === ticket) {
        pendingDictation = null;
        dictationStore.set((s) => ({ ...DICTATION_IDLE, lastText: s.lastText }));
      }
      return;
    }
    if (current?.ticket !== ticket) return;
    const stopWanted = current.stopWanted;
    pendingDictation = null;
    dictationStore.set((s) => ({ ...DICTATION_IDLE, lastText: s.lastText }));
    const run = ready.dictation.start(sessionId, as);
    if (stopWanted) void ready.dictation.stop();
    await run;
  };

  const stopDictation = async (): Promise<string | null> => {
    if (impl !== null) return impl.dictation.stop();
    if (pendingDictation !== null) pendingDictation.stopWanted = true;
    return null;
  };

  const cancelDictation = (): void => {
    if (impl !== null) {
      impl.dictation.cancel();
      return;
    }
    dictationTicket += 1;
    pendingDictation = null;
    dictationStore.set((s) => ({ ...DICTATION_IDLE, lastText: s.lastText }));
  };

  const turnOnVoiceChat = (sessionId: string): string | null => {
    if (impl !== null) return impl.voiceChat.turnOn(sessionId);
    if (deps.canUseKey?.() === false) return null;
    if (dictationBusy()) {
      return "Finish the dictation first: voice chat and dictation share the microphone.";
    }
    const cur = voiceChatStore.get();
    if (cur.on && cur.owner === sessionId) return null;
    voiceChatTicket += 1;
    const ticket = voiceChatTicket;
    pendingVoiceChat = { ticket, sessionId };
    voiceChatStore.set({
      ...VOICE_CHAT_OFF,
      on: true,
      owner: sessionId,
      phase: "listening",
      micPending: true,
      lastSpoken: cur.lastSpoken,
      spoken: cur.spoken,
    });
    void loadRuntime("voice chat", () => {
      turnOnVoiceChat(sessionId);
      return Promise.resolve();
    }).then((ready) => {
      if (ready === null) {
        if (pendingVoiceChat?.ticket === ticket) {
          pendingVoiceChat = null;
          voiceChatStore.set((s) => ({
            ...VOICE_CHAT_OFF,
            lastSpoken: s.lastSpoken,
            spoken: s.spoken,
          }));
        }
        return;
      }
      if (pendingVoiceChat?.ticket !== ticket) return;
      pendingVoiceChat = null;
      voiceChatStore.set((s) => ({
        ...VOICE_CHAT_OFF,
        lastSpoken: s.lastSpoken,
        spoken: s.spoken,
      }));
      ready.voiceChat.turnOn(sessionId);
    });
    return null;
  };

  const turnOffVoiceChat = (reason?: string): void => {
    if (impl !== null) {
      impl.voiceChat.turnOff(reason);
      return;
    }
    voiceChatTicket += 1;
    pendingVoiceChat = null;
    voiceChatStore.set((s) => ({
      ...VOICE_CHAT_OFF,
      note: reason ?? null,
      lastSpoken: s.lastSpoken,
      spoken: s.spoken,
    }));
  };

  const narrateLast = async (sessionId: string): Promise<void> => {
    if (impl !== null) return impl.narration.narrateLast(sessionId);
    narrationStore.set((s) => ({ ...s, phase: "summarizing", note: null }));
    const ready = await loadRuntime("narration", () => narrateLast(sessionId));
    if (ready === null) {
      narrationStore.set((s) => ({ ...s, phase: "idle", current: null }));
      throw new Error(chunkLoadStore.get().error ?? "Could not load narration");
    }
    narrationStore.set((s) => ({ ...s, phase: "idle", current: null }));
    return ready.narration.narrateLast(sessionId);
  };

  const dictation = {
    store: dictationStore,
    placed: placedStore,
    start: startDictation,
    stop: stopDictation,
    cancel: cancelDictation,
    stopForClose(): void {
      if (impl !== null) impl.dictation.stopForClose();
      else cancelDictation();
    },
    queueStart(sessionId: string, as: "ui" | "debug-api" = "ui"): boolean {
      if (impl !== null) return impl.dictation.queueStart(sessionId, as);
      return false;
    },
    setCaret(sessionId: string, start: number, end: number): void {
      if (impl !== null) impl.dictation.setCaret(sessionId, start, end);
      else bufferedCarets.set(sessionId, { start, end });
    },
    bindCaretReader(sessionId: string, read: CaretReader): () => void {
      if (impl !== null) return impl.dictation.bindCaretReader(sessionId, read);
      bufferedReaders.set(sessionId, read);
      return () => {
        if (bufferedReaders.get(sessionId) === read) bufferedReaders.delete(sessionId);
        realReaderUnbinds.get(read)?.();
      };
    },
    dismiss(): void {
      if (impl !== null) {
        impl.dictation.dismiss();
        return;
      }
      dictationStore.set((s) =>
        s.phase === "error" || s.note !== null
          ? { ...s, phase: s.phase === "error" ? "idle" : s.phase, error: null, note: null }
          : s,
      );
    },
    busy: dictationBusy,
  };
  readDictationBusy = () => dictation.busy();

  const narration = {
    store: narrationStore,
    stopAll(): void {
      pendingReadAlouds.length = 0;
      pendingNarrationTurns.length = 0;
      if (impl !== null) impl.narration.stopAll();
      else narrationStore.set((s) => ({ ...s, queue: [], phase: "idle", current: null }));
    },
    skip(): void {
      impl?.narration.skip();
    },
    narrateLast,
  };

  const voiceChat = {
    store: voiceChatStore,
    prompt: {
      store: promptStore,
      cancel(): void {
        impl?.voiceChat.prompt.cancel();
      },
      ended(): void {
        impl?.voiceChat.prompt.ended();
      },
      endOnError(thrown: unknown, active: boolean): boolean {
        return impl?.voiceChat.prompt.endOnError(thrown, active) ?? false;
      },
      end(): void {
        impl?.voiceChat.prompt.end();
      },
      reconcile(): void {
        impl?.voiceChat.prompt.reconcile();
      },
      prepare(callbackId: string, text: string): Promise<string> {
        return impl?.voiceChat.prompt.prepare(callbackId, text) ?? Promise.resolve(text);
      },
      speak(callbackId: string | undefined, text: string, voice: string | null) {
        if (impl === null) return Promise.reject(new Error("Voice module is not loaded."));
        return impl.voiceChat.prompt.speak(callbackId, text, voice);
      },
      played(callbackId: string): void {
        impl?.voiceChat.prompt.played(callbackId);
      },
      ready(): boolean {
        return impl?.voiceChat.prompt.ready() ?? false;
      },
      start(sampleRate: number, language: string | null) {
        if (impl === null) return Promise.reject(new Error("Voice module is not loaded."));
        return impl.voiceChat.prompt.start(sampleRate, language);
      },
      answer(streamId: string) {
        return (
          impl?.voiceChat.prompt.answer(streamId) ??
          Promise.resolve({ again: null, note: "Answer on the card." })
        );
      },
    },
    question: {
      store: questionStore,
      cancel(): void {
        impl?.voiceChat.question.cancel();
      },
      prepare(callbackId: string, text: string): string {
        return impl?.voiceChat.question.prepare(callbackId, text) ?? text;
      },
      played(callbackId: string): void {
        impl?.voiceChat.question.played(callbackId);
      },
      ready(): boolean {
        return impl?.voiceChat.question.ready() ?? false;
      },
      unread(queued: (callbackId: string) => boolean): void {
        impl?.voiceChat.question.unread(queued);
      },
      listening(): void {
        impl?.voiceChat.question.listening();
      },
      hear(streamId: string) {
        return (
          impl?.voiceChat.question.hear(streamId) ??
          Promise.resolve({ again: null, said: null, note: "" })
        );
      },
      reconcile(): void {
        impl?.voiceChat.question.reconcile();
      },
    },
    turnOn: turnOnVoiceChat,
    turnOff: turnOffVoiceChat,
    endTurn(): Promise<void> {
      return impl?.voiceChat.endTurn() ?? Promise.resolve();
    },
    press(sessionId: string): string | null {
      if (impl !== null) return impl.voiceChat.press(sessionId);
      const s = voiceChatStore.get();
      if (!s.on || s.owner !== sessionId) return turnOnVoiceChat(sessionId);
      return null;
    },
    owns(sessionId: string): boolean {
      const s = voiceChatStore.get();
      return s.on && s.owner === sessionId;
    },
    active(): boolean {
      return voiceChatStore.get().on;
    },
  };

  const prompts = {
    resume(): void {
      impl?.prompts.resume();
    },
    stop(): void {
      pendingPromptAsks.length = 0;
      impl?.prompts.stop();
    },
    watch(): () => void {
      if (impl !== null) return impl.prompts.watch();
      promptWatchActive = true;
      const seen = new Set(Object.keys(deps.core.store.get().asks));
      facadePromptStop = deps.core.listen((event) => {
        if (impl !== null || event.type !== "askOpened" || seen.has(event.ask.id)) return;
        seen.add(event.ask.id);
        if (seen.size > 1000) seen.delete(seen.values().next().value ?? "");
        const mode = accessibility.store.get();
        if (mode.promptSound !== "off" || mode.readPrompts === "always") {
          pendingPromptAsks.push(event.ask);
          void loadRuntime("Prompt audio");
        }
      });
      return () => {
        promptWatchActive = false;
        pendingPromptAsks.length = 0;
        facadePromptStop?.();
        facadePromptStop = null;
        realPromptUnwatch?.();
        realPromptUnwatch = null;
      };
    },
  };

  return {
    settings,
    dictation,
    narration,
    voiceChat,
    prompts,
    chunkLoad: {
      store: chunkLoadStore,
      dismiss(): void {
        chunkLoadStore.set(VOICE_CHUNK_IDLE);
        failedRetry = null;
      },
    },
    ensureLoaded: async (label = "voice"): Promise<boolean> => (await loadRuntime(label)) !== null,
    retryLoad: async (): Promise<void> => {
      if (failedRetry !== null) {
        const retry = failedRetry;
        failedRetry = null;
        await retry();
      } else {
        await loadRuntime("Voice");
      }
    },
    isLoaded: (): boolean => impl !== null,
    stopForClose(): void {
      stopCoreWatch();
      stopSettingsWatch();
      if (impl !== null) {
        impl.stopForClose();
        return;
      }
      turnOffVoiceChat();
      narration.stopAll();
      prompts.stop();
      dictation.stopForClose();
    },
  };
}

/** The voice controllers. */
export type Voice = ReturnType<typeof createVoice>;
