/**
 * Narration: **opt-in, off by default**.
 * When on, a turn of the selected session that completes while the app runs is read aloud: Vibe's own
 * narrator summary (`narration/summarize` through the gate) -> Voxtral TTS from the core
 * (`speech` `speak`, key from the vault) -> Web Audio. Vibe's narrator is a CLI-side component (the
 * app-server only summarizes; the CLI speaks), so the speaking part is ours.
 * Depends on: coreStore, shellStore (selected session), voiceSettings, the transcript entry helpers,
 * features/voice/audioPlayer (`Player`). Called by: createApp (via `createVoice`), the narration bar,
 * the `ui/voice` route.
 * Invariants:
 * - History present when a session is first seen is never narrated (only turns finishing now).
 * - Never while dictating: `dictating()` true -> the turn is skipped (state says so); a dictation start
 *   calls `stopAll`. Never a session in voice chat: voice chat speaks those turns itself.
 * - One sound at a time; a queue of at most 3; Stop clears it, Skip plays the next.
 * - What is sent to Mistral is the user's text and the turn's answer (capped), as the CLI narrator does.
 */
import type { CoreEvent, JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { Player } from "../features/voice/audioPlayer";
import { stripEmphasis } from "../features/voice/voiceChatText";
import {
  type Obj,
  arr,
  messageText,
  obj,
  roleOf,
  str,
  turnIdOf,
} from "../features/transcript/model/entries";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";
import type { VoiceSettingsStore } from "./voiceSettings";

/** Longest assistant text sent for one summary (the CLI narrator sends the whole turn; we cap it). */
export const MAX_ANSWER_CHARS = 8_000;
/** Most turns waiting to be narrated. */
const MAX_QUEUE = 3;

/** One turn to narrate. */
export interface NarrationItem {
  sessionId: string;
  turnId: string;
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

/** The user text and the answer of `turnId` from a session's history. */
export function turnTexts(
  history: readonly JsonValue[],
  turnId: string,
): { user: string; answer: string } {
  let user = "";
  const answer: string[] = [];
  for (const raw of history) {
    const entry = obj(raw);
    if (entry === null || turnIdOf(entry) !== turnId) continue;
    const role = roleOf(entry);
    if (role === "user" && user === "" && entry["source"] !== "harness") user = messageText(entry);
    else if (role === "assistant") answer.push(messageText(entry));
  }
  return { user, answer: answer.join("\n\n").slice(0, MAX_ANSWER_CHARS) };
}

/** Plain words for speech: code blocks, links and Markdown marks removed. Adapted from ShellX voice chat. */
export function speakable(text: string): string {
  const plain = text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    // marks only at line starts and word edges, so `snake_case` and `a > b` survive (was: every `*_#>|~`
    // removed).
    .replace(/^[ \t]*(?:#{1,6}[ \t]+|>[ \t]?|[-*+][ \t]+)/gm, "")
    .replace(/^[ \t]*\|.*$/gm, " ");
  return stripEmphasis(plain).replace(/\s+/g, " ").trim().slice(0, 2_000);
}

function decode(base64: string): Uint8Array {
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
  return out;
}

/** Turn ids of a session's completed turns. */
function completedTurns(state: Obj | null | undefined): string[] {
  return arr(state?.["turns"]).flatMap((t) => {
    const turn = obj(t);
    const id = str(turn?.["id"] ?? null);
    return turn?.["status"] === "completed" && id !== null ? [id] : [];
  });
}

/** Creates the narration controller. */
export function createNarration(deps: {
  core: CoreController;
  shell: Shell;
  settings: VoiceSettingsStore;
  player: Player;
  canUseKey?: (() => boolean) | undefined;
  /** True while a dictation runs (narration then stays quiet). */
  dictating: () => boolean;
  /** True when the session is in voice chat (voice chat speaks its turns, narration stays out). */
  voiceChat?: (sessionId: string) => boolean;
}) {
  const { core, shell, settings, player } = deps;
  const store = createStore<NarrationState>(NARRATION_IDLE);
  /** Completed turns already seen, per session (the first sight records history without narrating). */
  const seen = new Map<string, Set<string>>();
  let working = false;
  let generation = 0;
  let noteTimer: ReturnType<typeof setTimeout> | null = null;
  /** Shows a note for 8 seconds (a skipped or failed turn). */
  const setNote = (note: string): void => {
    store.set((s) => ({ ...s, note }));
    if (noteTimer !== null) clearTimeout(noteTimer);
    noteTimer = setTimeout(() => {
      noteTimer = null;
      store.set((s) => (s.note === note ? { ...s, note: null } : s));
    }, 8_000);
  };

  const historyOf = (sessionId: string): readonly JsonValue[] =>
    arr(core.store.get().sessions[sessionId]?.state?.["history"]);

  /** A key change discards pending audio and restores the idle bar without an error note. */
  const quietOnKeyLoss = (): boolean => {
    if (deps.canUseKey?.() !== false) return false;
    generation += 1;
    player.stop();
    store.set((s) => ({ ...s, phase: "idle", current: null, queue: [], note: null }));
    return true;
  };

  /** Summarizes, speaks and plays one turn. With `origin: "debug-api"`, the speak carries that label. */
  const narrate = async (item: NarrationItem, origin: "ui" | "debug-api"): Promise<void> => {
    if (deps.canUseKey?.() === false) return;
    const mine = generation;
    const opts = origin === "debug-api" ? { origin } : undefined;
    const { user, answer } = turnTexts(historyOf(item.sessionId), item.turnId);
    store.set((s) => ({
      ...s,
      phase: "summarizing",
      current: { ...item, summary: null },
      note: null,
    }));
    let summarized;
    try {
      summarized = await core.run(
        {
          type: "call",
          method: "narration/summarize",
          params: { sessionId: item.sessionId, userMessage: user, assistantText: answer },
        },
        opts,
      );
    } catch (thrown) {
      if (quietOnKeyLoss()) return;
      throw thrown;
    }
    const value = summarized.type === "reply" ? obj(summarized.result) : null;
    const summary = speakable(str(value?.["summary"] ?? null) ?? "");
    if (mine !== generation || quietOnKeyLoss()) return;
    if (summary === "") {
      setNote("Vibe gave no summary for this turn.");
      return;
    }
    store.set((s) => ({ ...s, phase: "speaking", current: { ...item, summary } }));
    const voice = settings.store.get().voice;
    let spoken;
    try {
      spoken = await core.run(
        { type: "speech", request: { op: "speak", text: summary, voice } },
        opts,
      );
    } catch (thrown) {
      if (quietOnKeyLoss()) return;
      throw thrown;
    }
    if (quietOnKeyLoss()) return;
    if (mine !== generation || spoken.type !== "speech" || spoken.result.kind !== "speech") return;
    const audio = decode(spoken.result.audio);
    store.set((s) => ({ ...s, phase: "playing" }));
    let played;
    try {
      played = await player.play(audio);
    } catch (thrown) {
      if (quietOnKeyLoss()) return;
      throw thrown;
    }
    if (quietOnKeyLoss()) return;
    if (mine !== generation) return;
    store.set((s) => ({
      ...s,
      last: { chars: summary.length, bytes: audio.length, seconds: played.seconds, summary },
    }));
  };

  /** Plays the queue one by one. */
  const pump = async (): Promise<void> => {
    if (working) return;
    working = true;
    try {
      for (;;) {
        const next = store.get().queue[0];
        if (next === undefined) break;
        store.set((s) => ({ ...s, queue: s.queue.slice(1) }));
        try {
          await narrate(next, "ui");
        } catch (thrown) {
          if (!quietOnKeyLoss()) setNote(`Narration failed: ${toCallError(thrown).error.message}`);
        }
      }
    } finally {
      working = false;
      store.set((s) => ({ ...s, phase: "idle", current: null }));
    }
  };

  const enqueue = (item: NarrationItem): void => {
    if (deps.canUseKey?.() === false) return;
    if (deps.voiceChat?.(item.sessionId) === true) return;
    if (deps.dictating()) {
      setNote("A turn finished while you were dictating; it was not read aloud.");
      return;
    }
    store.set((s) => ({ ...s, queue: [...s.queue, item].slice(-MAX_QUEUE) }));
    void pump();
  };

  /** Notices turns that completed since the last look. */
  const scan = (): void => {
    const sessions = core.store.get().sessions;
    for (const [id, entry] of Object.entries(sessions)) {
      const done = completedTurns(entry.state);
      const known = seen.get(id);
      if (known === undefined) {
        // First sight (history load): remember, never narrate.
        if (entry.state !== null) seen.set(id, new Set(done));
        continue;
      }
      for (const turnId of done) {
        if (known.has(turnId)) continue;
        known.add(turnId);
        if (settings.store.get().narration && shell.store.get().selected === id) {
          enqueue({ sessionId: id, turnId });
        }
      }
    }
  };
  core.store.subscribe(scan);
  scan();
  core.listen((event: CoreEvent) => {
    if (event.type === "snapshot") scan();
  });

  /** Stops the sound and clears the queue. */
  const stopAll = (): void => {
    generation += 1;
    player.stop();
    store.set((s) => ({ ...s, queue: [], phase: "idle", current: null }));
  };

  return {
    store,
    stopAll,
    /** Stops the current sound; the next queued turn plays. */
    skip(): void {
      generation += 1;
      player.stop();
    },
    /**
     * Narrates the session's last completed turn now (also the `ui/voice` route). Rejects with the failure.
     */
    async narrateLast(sessionId: string, origin: "ui" | "debug-api"): Promise<void> {
      const turns = completedTurns(core.store.get().sessions[sessionId]?.state);
      const turnId = turns[turns.length - 1];
      if (turnId === undefined) throw new Error("this session has no finished turn yet");
      try {
        await narrate({ sessionId, turnId }, origin);
      } finally {
        store.set((s) => ({ ...s, phase: "idle", current: null }));
      }
    },
  };
}

/** The narration controller. */
export type Narration = ReturnType<typeof createNarration>;
