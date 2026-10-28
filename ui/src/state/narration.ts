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
 * - One sound at a time; automatic narration queues at most 3; at most 2 requested readings outstanding.
 *   Stop clears it, Skip skips the whole current reading and plays the next.
 * - What is sent to Mistral is the user's text and the turn's answer (capped), as the CLI narrator does.
 */
import type { CoreEvent, JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { Player } from "../features/voice/audioPlayer";
import { speakable, readingChunks } from "../features/voice/speechText";
export { speakable } from "../features/voice/speechText";
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
const MAX_ANSWER_CHARS = 8_000;
/** Most turns waiting to be narrated. */
const MAX_QUEUE = 3;
/** Requested readings include speech being prepared, queued audio and the current reading. */
const MAX_READINGS = 2;

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
  let readingGeneration = 0;
  let preparing: Promise<void> = Promise.resolve();
  const preparingReadings = new Set<object>();
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

  /** Summarizes, speaks and plays one turn. */
  const narrate = async (item: NarrationItem): Promise<void> => {
    if (deps.canUseKey?.() === false) return;
    const mine = generation;
    const { user, answer } = turnTexts(historyOf(item.sessionId), item.turnId);
    store.set((s) => ({
      ...s,
      phase: "summarizing",
      current: { ...item, summary: null },
      note: null,
    }));
    let summary: string;
    if (item.reading !== undefined) {
      summary = speakable(item.reading.text, 10_000, true);
    } else {
      let summarized;
      try {
        summarized = await core.run({
          type: "call",
          method: "narration/summarize",
          params: { sessionId: item.sessionId, userMessage: user, assistantText: answer },
        });
      } catch (thrown) {
        if (quietOnKeyLoss()) return;
        throw thrown;
      }
      const value = summarized.type === "reply" ? obj(summarized.result) : null;
      summary = speakable(str(value?.["summary"] ?? null) ?? "");
    }
    if (mine !== generation || quietOnKeyLoss()) return;
    if (summary === "") {
      setNote("Vibe gave no summary for this turn.");
      return;
    }
    store.set((s) => ({ ...s, phase: "speaking", current: { ...item, summary } }));
    const voice = item.reading === undefined ? settings.store.get().voice : item.reading.voice;
    const chunks = item.reading?.chunks ?? [summary];
    for (const [index, chunk] of chunks.entries()) {
      if (mine !== generation) return;
      store.set((s) => ({ ...s, phase: "speaking" }));
      let encoded = index === 0 ? item.reading?.firstAudio : undefined;
      if (encoded === undefined) {
        let spoken;
        try {
          spoken = await core.run({ type: "speech", request: { op: "speak", text: chunk, voice } });
        } catch (thrown) {
          if (quietOnKeyLoss()) return;
          throw thrown;
        }
        if (quietOnKeyLoss()) return;
        if (mine !== generation || spoken.type !== "speech" || spoken.result.kind !== "speech")
          return;
        encoded = spoken.result.audio;
      }
      const audio = decode(encoded);
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
        last: {
          chars: Array.from(chunk).length,
          bytes: audio.length,
          seconds: played.seconds,
          summary: chunk,
        },
      }));
    }
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
          await narrate(next);
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
    store.set((s) => {
      const automatic = s.queue.filter((queued) => queued.reading === undefined);
      const oldest = automatic.length >= MAX_QUEUE ? automatic[0] : undefined;
      return { ...s, queue: [...s.queue.filter((queued) => queued !== oldest), item] };
    });
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
  /** Prepare actual speech before accepting a reading, so an unavailable voice leaves the bar unchanged. */
  const prepareReading = async (
    event: Extract<CoreEvent, { type: "readAloud" }>,
    mine: number,
  ): Promise<void> => {
    const chunks = readingChunks(event.text, event.truncated);
    const voice = settings.store.get().voice;
    let firstAudio: string | undefined;
    try {
      const first = chunks[0];
      if (first !== undefined && mine === readingGeneration && deps.canUseKey?.() !== false) {
        const spoken = await core.run({
          type: "speech",
          request: { op: "speak", text: first, voice },
        });
        if (spoken.type === "speech" && spoken.result.kind === "speech")
          firstAudio = spoken.result.audio;
      }
    } catch {
      // The tool receives the failure sentence; a failed request adds no bar or note.
    }
    const available =
      firstAudio !== undefined && mine === readingGeneration && deps.canUseKey?.() !== false;
    try {
      const receipt = await core.run({
        type: "speech",
        request: { op: "readingReady", requestId: event.requestId, available },
      });
      if (
        !available ||
        mine !== readingGeneration ||
        receipt.type !== "speech" ||
        receipt.result.kind !== "reading" ||
        receipt.result.status !== "accepted" ||
        firstAudio === undefined
      )
        return;
      const item: NarrationItem = {
        sessionId: event.sessionId,
        turnId: "",
        reading: { text: event.text, title: event.title, chunks, firstAudio, voice },
      };
      store.set((s) => ({ ...s, queue: [...s.queue, item] }));
      void pump();
    } catch {
      // A window receipt that could not reach the core must never start a late reading.
    }
  };
  core.listen((event: CoreEvent) => {
    if (event.type === "snapshot") scan();
    if (event.type === "readAloud") {
      const state = store.get();
      const outstanding =
        preparingReadings.size +
        (state.current?.reading === undefined ? 0 : 1) +
        state.queue.filter((item) => item.reading !== undefined).length;
      if (outstanding >= MAX_READINGS) {
        void core
          .run({
            type: "speech",
            request: {
              op: "readingReady",
              requestId: event.requestId,
              available: false,
              busy: true,
            },
          })
          .catch(() => {
            // A disconnected receipt cannot start playback.
          });
        return;
      }
      const mine = readingGeneration;
      const slot = {};
      preparingReadings.add(slot);
      preparing = preparing.then(async () => {
        try {
          await prepareReading(event, mine);
        } finally {
          preparingReadings.delete(slot);
        }
      });
    }
  });

  /** Stops the sound and clears the queue. */
  const stopAll = (): void => {
    readingGeneration += 1;
    preparingReadings.clear();
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
    async narrateLast(sessionId: string): Promise<void> {
      const turns = completedTurns(core.store.get().sessions[sessionId]?.state);
      const turnId = turns[turns.length - 1];
      if (turnId === undefined) throw new Error("this session has no finished turn yet");
      try {
        await narrate({ sessionId, turnId });
      } finally {
        store.set((s) => ({ ...s, phase: "idle", current: null }));
      }
    },
  };
}
