/**
 * Shared voice state shapes and idle defaults for the lazy facade, hooks, and UI components.
 * Depends on: voiceChatState, protocol/core types. Called by: state/voice, state/voiceImpl,
 * features/voice/hooks, SessionRow, ApprovalAsk, Composer.
 * Invariants: pure data and types only; never imports features/voice/*.
 */
import type { DictationMode } from "../protocol/core";
export {
  VOICE_CHAT_OFF,
  type VoiceChatState,
  type VoiceChatPhase,
  type TurnEndedBy,
  type SpokenKind,
  SENTENCE_GAP_MS,
} from "./voiceChatState";

/** Where a dictation is. */
export type DictationPhase = "idle" | "starting" | "recording" | "transcribing" | "error";

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

/** One turn to narrate. */
export interface NarrationItem {
  sessionId: string;
  turnId: string;
  /** Present only for a requested one-time reading, never an automatic turn summary. */
  reading?: {
    text: string;
    title: string;
    chunks: readonly string[];
    firstAudio: string;
    voice: string | null;
  };
}

/** The narration state the bar and the route read. */
export interface NarrationState {
  phase: "idle" | "summarizing" | "speaking" | "playing";
  current: (NarrationItem & { summary: string | null }) | null;
  queue: readonly NarrationItem[];
  /** The last sound played. */
  last: { chars: number; bytes: number; seconds: number; summary: string } | null;
  /** Why a turn was skipped or failed (shown briefly). */
  note: string | null;
}

/** The state with nothing going on. */
export const NARRATION_IDLE: NarrationState = {
  phase: "idle",
  current: null,
  queue: [],
  last: null,
  note: null,
};

/** The permission prompt answered by voice: its core ticket, answer phrase, spoken text and whether it has played. */
export interface VoicePromptState {
  callbackId: string;
  ticket: string;
  phrase: string;
  text: string;
  /**
   * The short prompt spoken when Mistral's safety filter refuses `text`: the card's question (from the effect kind and
   * tool, never the agent's own words) and the answer phrase. `null`: the action cannot be named, so the card stays.
   */
  fallback: string | null;
  ready: boolean;
}

/** The question card being answered by voice. */
export interface VoiceQuestionState {
  sessionId: string;
  callbackId: string;
  /** The question being asked now (0-based). */
  index: number;
  /** Ready to listen: its reading played. */
  ready: boolean;
  /** The microphone is open for this answer. */
  listening: boolean;
}

/** What voice chat does after one spoken answer: read `again` and listen once more, and/or say `said`. */
export interface VoiceQuestionReply {
  again: string | null;
  said: string | null;
  /** What was heard when it was not understood ("I heard “glue”."): spoken first and shown on the strip. */
  heard?: string;
}

/** Spoken and shown when a card's voice-answer window really ends (question and permission cards alike). */
export const VOICE_ANSWER_ENDED =
  "Voice answers for this question ended. Answer on the card in the chat.";

/** Status of loading the on-demand voice runtime chunk. */
export interface VoiceChunkLoadState {
  phase: "idle" | "loading" | "error";
  error: string | null;
  actionLabel: string | null;
}

/** Idle chunk-load status. */
export const VOICE_CHUNK_IDLE: VoiceChunkLoadState = {
  phase: "idle",
  error: null,
  actionLabel: null,
};
