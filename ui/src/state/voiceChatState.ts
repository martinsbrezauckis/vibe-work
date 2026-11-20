/** Voice-chat view state shared by the controller, controls and scripted tests. No I/O. */
import type { VoiceLatency } from "../features/voice/voiceLatency";
import type { Accessibility } from "./accessibility";
import type { Player } from "../features/voice/audioPlayer";
import type { PrepareCapture } from "../features/voice/pcmCapture";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import type { Toasts } from "./toastStore";
import type { VoiceSettingsStore } from "./voiceSettings";
import type { JsonValue, SpeechResult } from "../protocol/core";

/** Where voice chat is. `error`: stopped with a reason, waiting for a press (the mode stays on). */
export type VoiceChatPhase =
  "off" | "listening" | "sending" | "thinking" | "waiting" | "speaking" | "error";

/** What ended the user's last turn. */
export type TurnEndedBy = "button" | "pause" | "limit";

/** The state the button, the strip, the session row and the route read. */
export interface VoiceChatState {
  on: boolean;
  owner: string | null;
  phase: VoiceChatPhase;
  /** The microphone is opening (an OS permission prompt may be up). */
  micPending: boolean;
  /**
   * Listening, but no sound (a sample other than exact digital silence) has come from the microphone yet. The strip
   * says Listening only once it has; after `NO_SOUND_MS` without sound voice chat stops and says why.
   */
  awaitingSound: boolean;
  /** The post-pause microphone is still collecting words for the next message. */
  carryOpen: boolean;
  /** A spoken reply paused the microphone and this listening stretch resumed it. */
  resumed: boolean;
  /** Live words of the current listening stretch. */
  partial: string;
  /** What the user said last (sent as a voice turn). */
  heard: string | null;
  endedBy: TurnEndedBy | null;
  /** The last thing spoken: its text, length and decoded seconds. */
  lastSpoken: { text: string; chars: number; seconds: number; kind: SpokenKind } | null;
  /** How many texts were spoken since the app started (tests and the route). */
  spoken: number;
  note: string | null;
  error: string | null;
  /** Stage boundaries of the last voice turn (Debug API, no audio or text). */
  latency: VoiceLatency | null;
}

/** What a spoken text was. */
export type SpokenKind = "answer" | "prompt" | "failed";

/** The state with voice chat off. */
export const VOICE_CHAT_OFF: VoiceChatState = {
  on: false,
  owner: null,
  phase: "off",
  micPending: false,
  awaitingSound: false,
  carryOpen: false,
  resumed: false,
  partial: "",
  heard: null,
  endedBy: null,
  lastSpoken: null,
  spoken: 0,
  note: null,
  error: null,
  latency: null,
};

/** Existing controller dependencies; type-only references create no runtime ownership. */
export interface VoiceChatDeps {
  accessibility?: Accessibility;
  promptBlocked?: () => boolean;
  core: CoreController;
  composer: Composer;
  settings: VoiceSettingsStore;
  toasts: Toasts;
  prepare: PrepareCapture;
  player: Player;
  canUseKey?: (() => boolean) | undefined;
  /** True while a dictation runs (voice chat then cannot start). */
  dictating: () => boolean;
  /** Called when voice chat starts (narration stops). */
  onStart: () => void;
  /** A session's title for messages. */
  titleOf: (sessionId: string) => string;
  /** Sends a card's answer as the card's own buttons do (default: the core's `answerCallback`). */
  answer?: (sessionId: string, callbackId: string, output: JsonValue) => Promise<boolean>;
  now?: () => number;
}

/** One queued utterance and its optional prefetched speech result. */
export type VoiceChatUtterance = {
  text: string;
  kind: SpokenKind;
  turnId?: string;
  callbackId?: string;
  repeat?: boolean;
  ready?: Promise<SpeechResult>;
};

/** How long after a send voice chat waits for the turn to show before it listens again. */
export const EXPECT_MS = 15_000;
/**
 * How long an opened microphone may send only digital silence before voice chat stops and says so. A working
 * microphone sends its noise floor within a block; a muted, unplugged-in-software or wrongly routed one sends
 * zeros or nothing, and the person would otherwise talk to a strip that says Listening.
 */
export const NO_SOUND_MS = 4_000;
/** How long the session must look idle before the microphone opens again. */
export const LISTEN_SETTLE_MS = 300;
/**
 * The pause between two read sentences. Each sentence is its own synthesized clip whose end carries almost no
 * silence, so without it the next sentence starts sooner than a comma's pause inside one clip.
 */
export const SENTENCE_GAP_MS = 300;
