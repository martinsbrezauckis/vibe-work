/**
 * Voice as one object for the app: the settings, dictation and narration controllers, wired so that
 * starting a dictation silences narration and narration never speaks over a dictation. Voice chat adds
 * it shares the one player (one sound at a time), stops narration when it starts, and narration leaves its session
 * alone; dictation and voice chat never hold the microphone together (each refuses while the other runs).
 * Depends on: voiceSettings, dictation, narration, voiceChat, features/voice (real capture and player by default).
 * Called by: createApp; tests pass fakes for `prepare` and `player`.
 * Invariants: without a core there is no voice (`createApp` passes `null`).
 */
import { createWebAudioPlayer, type Player } from "../features/voice/audioPlayer";
import { type PrepareCapture, prepareCapture } from "../features/voice/pcmCapture";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import { createDictation } from "./dictation";
import { createNarration } from "./narration";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import { createVoiceChat } from "./voiceChat";
import { createVoiceSettings } from "./voiceSettings";

/** Creates the voice controllers. */
export function createVoice(deps: {
  core: CoreController;
  composer: Composer;
  shell: Shell;
  toasts: Toasts;
  persist: boolean;
  prepare?: PrepareCapture;
  player?: Player;
  /** Live key readiness; omitted for development without a key service. */
  canUseKey?: () => boolean;
}) {
  const settings = createVoiceSettings(deps.persist);
  const player = deps.player ?? createWebAudioPlayer();
  const prepare = deps.prepare ?? prepareCapture;
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
    },
    blocked: () =>
      voiceChat?.active() === true
        ? "Voice chat is on: end it first, dictation and voice chat share the microphone."
        : null,
  });
  voiceChat = createVoiceChat({
    core: deps.core,
    composer: deps.composer,
    settings,
    toasts: deps.toasts,
    prepare,
    player,
    canUseKey: deps.canUseKey,
    dictating: () => dictation.busy(),
    onStart: () => {
      narration?.stopAll();
    },
    titleOf: (id) => {
      const title = deps.core.store.get().sessions[id]?.session["title"];
      return typeof title === "string" && title !== "" ? title : "another session";
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
  return { settings, dictation, narration, voiceChat };
}

/** The voice controllers. */
export type Voice = ReturnType<typeof createVoice>;
