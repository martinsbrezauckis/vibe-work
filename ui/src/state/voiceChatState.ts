/** Voice-chat view state shared by the controller, controls and scripted tests. No I/O. */
import type { VoiceLatency } from "../features/voice/voiceLatency";

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
  partial: "",
  heard: null,
  endedBy: null,
  lastSpoken: null,
  spoken: 0,
  note: null,
  error: null,
  latency: null,
};
