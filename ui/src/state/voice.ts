/**
 * Voice as one object for the app: the settings, dictation and narration controllers, wired so that
 * starting a dictation silences narration and narration never speaks over a dictation. Voice chat adds
 * it shares the one player (one sound at a time), stops narration when it starts, and narration leaves its session
 * alone; dictation and voice chat never hold the microphone together (each refuses while the other runs).
 * Depends on: voiceSettings, dictation, narration, voiceChat, features/voice (real capture and player by default).
 * Called by: createApp; tests pass fakes for `prepare` and `player`.
 * Invariants: without a core there is no voice (`createApp` passes `null`).
 */
import { createWebAudioPlayer, type Player, withOutputCheck } from "../features/voice/audioPlayer";
import { type PrepareCapture, prepareCapture } from "../features/voice/pcmCapture";
import type { JsonValue } from "../protocol/core";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import { createDictation } from "./dictation";
import { createNarration } from "./narration";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import { createVoiceChat } from "./voiceChat";
import { VOICE_ANSWER_ENDED } from "./voiceQuestion";
import { createAccessibility, type Accessibility } from "./accessibility";
import { createPromptAudio } from "./promptAudio";
import { createVoiceSettings } from "./voiceSettings";
import { detectMac } from "./shortcuts";

/** Creates the voice controllers. */
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
}) {
  const settings = createVoiceSettings(deps.persist);
  const accessibility = deps.accessibility ?? createAccessibility(deps.persist);
  let prompts: ReturnType<typeof createPromptAudio> | null = null;
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
  let narration: ReturnType<typeof createNarration> | null = null;
  let voiceChat: ReturnType<typeof createVoiceChat> | null = null;
  const dictation = createDictation({
    core: deps.core,
    composer: deps.composer,
    settings,
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
  });
  voiceChat = createVoiceChat({
    accessibility,
    promptBlocked: () =>
      deps.shell.store.get().settings.open ||
      deps.shell.store.get().selected !== voiceChat?.store.get().owner ||
      Object.values(deps.core.store.get().asks).some(
        (a) => a.surface === "native" || a.personOnly,
      ) ||
      document.querySelector('[role="dialog"][aria-modal="true"], [role="alertdialog"]') !== null,
    core: deps.core,
    composer: deps.composer,
    settings,
    toasts: deps.toasts,
    prepare,
    player,
    canUseKey: deps.canUseKey,
    ...(deps.answer === undefined ? {} : { answer: deps.answer }),
    dictating: () => dictation.busy(),
    onStart: () => {
      narration?.stopAll();
      prompts?.stop();
    },
    titleOf: (id) => {
      const title = deps.core.store.get().sessions[id]?.session["title"];
      return typeof title === "string" && title !== "" ? title : "another chat";
    },
  });
  narration = createNarration({
    core: deps.core,
    shell: deps.shell,
    settings,
    player,
    canUseKey: deps.canUseKey,
    dictating: () => dictation.busy(),
    voiceChat: (id) => voiceChat.owns(id),
  });
  // A session in voice chat sends voice turns, also when the user types and presses Enter.
  deps.composer.setVoiceTurns((id) => voiceChat.owns(id));
  prompts = createPromptAudio({
    core: deps.core,
    accessibility,
    settings,
    player,
    signal: withOutputCheck(deps.player ?? createWebAudioPlayer(), outputAvailable),
    busy: () => dictation.busy() || voiceChat.active() || narration.store.get().phase !== "idle",
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
  return { settings, dictation, narration, voiceChat, prompts, stopForClose };
}

/** The voice controllers. */
export type Voice = ReturnType<typeof createVoice>;
