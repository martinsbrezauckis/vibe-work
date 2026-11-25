/**
 * On-demand voice runtime wiring: connects dictation, voice chat, narration, prompt audio, Web Audio
 * playback, and PCM microphone capture. Loaded dynamically on first voice use (or preloaded in tests)
 * so startup first paint stays small.
 * Depends on: dictation, narration, voiceChat, promptAudio, features/voice/{audioPlayer,pcmCapture}.
 * Called by: state/voice.
 * Invariants: receives the facade's pre-created stores so React subscriptions and direct store writes
 * share one identity across lazy loading.
 */
import { createWebAudioPlayer, type Player, withOutputCheck } from "../features/voice/audioPlayer";
import { type PrepareCapture, prepareCapture } from "../features/voice/pcmCapture";
import type { JsonValue } from "../protocol/core";
import type { Accessibility } from "./accessibility";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import type { Store } from "./createStore";
import { createDictation } from "./dictation";
import { createNarration } from "./narration";
import { createPromptAudio } from "./promptAudio";
import type { Shell } from "./shellStore";
import { detectMac } from "./shortcuts";
import type { Toasts } from "./toastStore";
import { createVoiceChat } from "./voiceChat";
import type { VoiceSettingsStore } from "./voiceSettings";
import {
  type CaretPlacement,
  type DictationState,
  type NarrationState,
  VOICE_ANSWER_ENDED,
  type VoiceChatState,
  type VoicePromptState,
  type VoiceQuestionState,
} from "./voiceTypes";

/** Dependencies passed from the thin `state/voice` facade. */
export interface VoiceImplDeps {
  core: CoreController;
  composer: Composer;
  shell: Shell;
  toasts: Toasts;
  settings: VoiceSettingsStore;
  accessibility: Accessibility;
  prepare?: PrepareCapture | undefined;
  player?: Player | undefined;
  canUseKey?: (() => boolean) | undefined;
  answer?:
    ((sessionId: string, callbackId: string, output: JsonValue) => Promise<boolean>) | undefined;
  isDictating?: (() => boolean) | undefined;
}

/** Shared store instances allocated synchronously by the facade. */
export interface VoiceImplStores {
  dictationStore: Store<DictationState>;
  placedStore: Store<CaretPlacement>;
  narrationStore: Store<NarrationState>;
  voiceChatStore: Store<VoiceChatState>;
  promptStore: Store<VoicePromptState | null>;
  questionStore: Store<VoiceQuestionState | null>;
}

/** Builds the full voice runtime wired onto the shared facade stores. */
export function createVoiceImpl(deps: VoiceImplDeps, stores: VoiceImplStores) {
  // Every sound first asks the host whether it can be heard (the AppImage without a sound server cannot).
  const outputAvailable = async () => {
    const answer = await deps.core.run({ type: "speech", request: { op: "audioOutput" } });
    return !(
      answer.type === "speech" &&
      answer.result.kind === "audioOutput" &&
      !answer.result.available
    );
  };
  const player = withOutputCheck(deps.player ?? createWebAudioPlayer(), outputAvailable);
  // macOS: Apple's voice processing (what echo cancellation and noise suppression turn on in WebKit) attenuates
  // or silences the input, the start of speech most of all, so dictation and voice chat both record raw there.
  const prepare =
    deps.prepare ?? (detectMac() ? () => prepareCapture({ processing: false }) : prepareCapture);
  let prompts: ReturnType<typeof createPromptAudio> | null = null;
  let narration: ReturnType<typeof createNarration> | null = null;
  let voiceChat: ReturnType<typeof createVoiceChat> | null = null;

  const dictation = createDictation({
    core: deps.core,
    composer: deps.composer,
    settings: deps.settings,
    prepare,
    canUseKey: deps.canUseKey,
    onStart: () => {
      narration?.stopAll();
      prompts?.stop();
    },
    blocked: () =>
      voiceChat?.active() === true
        ? "Voice chat is on: end it first, dictation and voice chat share the microphone."
        : null,
    store: stores.dictationStore,
    placed: stores.placedStore,
  });

  voiceChat = createVoiceChat({
    accessibility: deps.accessibility,
    promptBlocked: () =>
      deps.shell.store.get().settings.open ||
      deps.shell.store.get().selected !== voiceChat?.store.get().owner ||
      Object.values(deps.core.store.get().asks).some(
        (a) => a.surface === "native" || a.personOnly,
      ) ||
      document.querySelector('[role="dialog"][aria-modal="true"], [role="alertdialog"]') !== null,
    core: deps.core,
    composer: deps.composer,
    settings: deps.settings,
    toasts: deps.toasts,
    prepare,
    player,
    canUseKey: deps.canUseKey,
    ...(deps.answer === undefined ? {} : { answer: deps.answer }),
    dictating: () => deps.isDictating?.() ?? dictation.busy(),
    onStart: () => {
      narration?.stopAll();
      prompts?.stop();
    },
    titleOf: (id) => {
      const title = deps.core.store.get().sessions[id]?.session["title"];
      return typeof title === "string" && title !== "" ? title : "another chat";
    },
    store: stores.voiceChatStore,
    promptStore: stores.promptStore,
    questionStore: stores.questionStore,
  });

  narration = createNarration({
    core: deps.core,
    shell: deps.shell,
    settings: deps.settings,
    player,
    canUseKey: deps.canUseKey,
    dictating: () => deps.isDictating?.() ?? dictation.busy(),
    voiceChat: (id) => voiceChat.owns(id),
    store: stores.narrationStore,
  });

  // A session in voice chat sends voice turns, also when the user types and presses Enter.
  deps.composer.setVoiceTurns((id) => voiceChat.owns(id));

  prompts = createPromptAudio({
    core: deps.core,
    accessibility: deps.accessibility,
    settings: deps.settings,
    player,
    signal: withOutputCheck(deps.player ?? createWebAudioPlayer(), outputAvailable),
    busy: () =>
      (deps.isDictating?.() ?? dictation.busy()) ||
      voiceChat.active() ||
      narration.store.get().phase !== "idle",
    owns: (id) => id !== null && voiceChat.owns(id),
  });

  deps.shell.store.subscribe(() => {
    // Permission answers belong to the visible card. Leaving its chat closes capture and hands it back;
    // the toast remains visible in the newly selected chat while voice chat speaks the same sentence.
    if (
      voiceChat.store.get().phase === "listening" &&
      voiceChat.prompt.store.get() !== null &&
      deps.shell.store.get().selected !== voiceChat.store.get().owner
    ) {
      voiceChat.prompt.ended();
      deps.toasts.feedback(VOICE_ANSWER_ENDED);
    } else voiceChat.prompt.reconcile();
  });

  const stopForClose = () => {
    voiceChat.turnOff();
    narration.stopAll();
    prompts.stop();
    dictation.stopForClose();
  };

  return { dictation, narration, voiceChat, prompts, stopForClose };
}

/** Loaded voice runtime controllers. */
export type VoiceImpl = ReturnType<typeof createVoiceImpl>;
