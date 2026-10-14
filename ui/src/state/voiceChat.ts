/**
 * Voice chat: one session at a time talks with Vibe by
 * voice. The loop: listen (microphone → the core's speech stream, the same Voxtral path as dictation) → the user's turn
 * ends (the button, or a pause: `features/voice/turnEnd`) → the words go out as a voice turn (the composer's
 * `sendSpoken`: the voice instruction rides in Vibe's context channel, never in the message) → thinking (an approval or
 * a question is spoken as a short prompt and waits for the card). Complete reply sentences stream through core
 * `speak` with the Settings voice while the model continues; the next sentence synthesizes during playback.
 * After the turn and its audio finish, listen again. Adapted from ShellX's voice chat, the state machine keeps one owner and
 * one active audio stream, resumes listening after speech, and keeps the mode on after recoverable errors.
 * Depends on: coreStore, composer (`sendSpoken`), voiceSettings, toastStore, voiceChatTurns, the voice features
 * (capture, player, turn end, words). Called by: state/voice (`createVoice`), the Voice chat button, the strip, the
 * session row tag, the `ui/voice` route.
 * Invariants:
 * - At most one owner session; turning it on elsewhere moves it (a toast says so). In memory only: a restart never
 *   reopens the microphone.
 * - The microphone is open only while listening (never while speaking: the reply cannot hear itself).
 * - Errors keep the mode on with the reason; only a lost or refused microphone, the owner session closing,
 *   or the user turning it off ends the mode. A turn that finishes after voice chat is off is never spoken.
 * - Every async step re-checks `gen` (bumped by on/off/move) so a late answer never acts on a newer state.
 * - Model output is untrusted: it is only turned into words (`spokenReply`) and spoken, never run.
 */
import type { CoreEvent, SpeechRequest, SpeechResult } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { Player } from "../features/voice/audioPlayer";
import { type PrepareCapture, micErrorText } from "../features/voice/pcmCapture";
import { type TurnEnd, createTurnEnd, pcmRms } from "../features/voice/turnEnd";
import { FAILED_SAID } from "../features/voice/voiceChatText";
import { type VoiceLatency, voiceLatency } from "../features/voice/voiceLatency";
import { type VoiceReplyBudget, createVoiceReplyStream } from "../features/voice/voiceReplyStream";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import { toBase64 } from "./dictation";
import { NEW_CHAT } from "./newChatKey";
import type { Toasts } from "./toastStore";
import {
  endedTurns,
  finalAnswer,
  openPrompts,
  runningAnswer,
  sessionBusy,
  sessionClosed,
  turnAnswers,
  voiceTurnId,
} from "./voiceChatTurns";
import type { VoiceSettingsStore } from "./voiceSettings";

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

/** How long after a send voice chat waits for the turn to show before it listens again. */
const EXPECT_MS = 15_000;
/** How long a handed-off owner may be missing from the model (its new snapshot on the way). */
const HANDOFF_WAIT_MS = 5_000;
/** How long the session must look idle before the microphone opens again. */
const LISTEN_SETTLE_MS = 300;

function decode(base64: string): Uint8Array {
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
  return out;
}

/** Creates the voice chat controller. */
export function createVoiceChat(deps: {
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
  now?: () => number;
}) {
  const { core, composer, settings, toasts, player } = deps;
  const now = deps.now ?? (() => Date.now());
  const store = createStore<VoiceChatState>(VOICE_CHAT_OFF);
  let gen = 0;
  let busy: "listen" | "send" | "speak" | null = null;
  let capture: { stop(): void } | null = null;
  let streamId: string | null = null;
  let chain: Promise<unknown> = Promise.resolve();
  let detector: TurnEnd | null = null;
  /** Audio received in the current listening stretch (ms): the turn-end rule's clock. */
  let heardMs = 0;
  let known = new Set<string>();
  let prompted = new Set<string>();
  let expecting: number | null = null;
  let expectTimer: ReturnType<typeof setTimeout> | null = null;
  let listenTimer: ReturnType<typeof setTimeout> | null = null;
  type Say = { text: string; kind: SpokenKind; turnId?: string; ready?: Promise<SpeechResult> };
  let queue: Say[] = [];
  let currentSpeech: Say | null = null;
  const streams = new Map<string, ReturnType<typeof createVoiceReplyStream>>();
  const speechBudgets = new Map<string, VoiceReplyBudget>();
  const mutedTurns = new Set<string>();
  let skipSpeech = 0;
  let lastLoudAt: number | null = null;

  const entryOf = (id: string | null) => (id === null ? undefined : core.store.get().sessions[id]);
  const speech = async (request: SpeechRequest): Promise<SpeechResult> => {
    const r = await core.run({ type: "speech", request });
    if (r.type !== "speech") throw new Error("unexpected answer to a speech request");
    return r.result;
  };
  const patch = (p: Partial<VoiceChatState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  const stamp = (stage: keyof VoiceLatency): void => {
    const t = store.get().latency;
    if (t !== null && t[stage] === null) patch({ latency: { ...t, [stage]: now() } });
  };

  /** Stops whatever runs (microphone, stream, speech) without changing the mode. */
  const halt = (): void => {
    capture?.stop();
    capture = null;
    const id = streamId;
    streamId = null;
    detector = null;
    if (id !== null) void speech({ op: "cancel", streamId: id }).catch(() => undefined);
    if (busy === "speak") player.stop();
    queue = [];
    currentSpeech = null;
    streams.clear();
    speechBudgets.clear();
    mutedTurns.clear();
    skipSpeech += 1;
    busy = null;
    expecting = null;
    if (expectTimer !== null) clearTimeout(expectTimer);
    expectTimer = null;
    if (listenTimer !== null) clearTimeout(listenTimer);
    listenTimer = null;
  };

  /** At most one sentence ahead: synthesize it while the current sentence plays. */
  const prepareSpeech = (item: Say): Promise<SpeechResult> => {
    if (item.ready !== undefined) return item.ready;
    if (item.kind === "answer") stamp("firstTtsRequest");
    item.ready = speech({ op: "speak", text: item.text, voice: settings.store.get().voice });
    // A prefetched request can fail before the player awaits it. The player still reports its error.
    void item.ready.catch(() => undefined);
    return item.ready;
  };
  const prefetch = (): void => {
    if (busy === "speak" && currentSpeech !== null && queue[0] !== undefined)
      void prepareSpeech(queue[0]);
  };

  /** Turns voice chat off (`reason`: shown on the strip and the button). */
  const turnOff = (reason: string | null = null): void => {
    if (!store.get().on) return;
    gen += 1;
    halt();
    store.set((s) => ({
      ...VOICE_CHAT_OFF,
      lastSpoken: s.lastSpoken,
      spoken: s.spoken,
      heard: s.heard,
      latency: s.latency,
      note: reason,
    }));
    // The reason is visible even though the composer's strip goes away with the mode.
    if (reason !== null) toasts.push({ kind: "warn", bellKind: "failure", text: reason });
  };

  const speakAll = async (): Promise<void> => {
    if (deps.canUseKey?.() === false) {
      turnOff();
      return;
    }
    const mine = gen;
    busy = "speak";
    while (queue.length > 0 && mine === gen) {
      if (deps.canUseKey?.() === false) {
        turnOff();
        return;
      }
      const item = queue.shift();
      if (item === undefined) break;
      currentSpeech = item;
      const ticket = skipSpeech;
      patch({ phase: "speaking" });
      try {
        const r = await prepareSpeech(item);
        if (deps.canUseKey?.() === false) {
          turnOff();
          return;
        }
        if (mine !== gen || ticket !== skipSpeech || r.kind !== "speech") break;
        prefetch();
        const played = await player.play(decode(r.audio), () => {
          if (mine === gen && ticket === skipSpeech && item.kind === "answer")
            stamp("firstAudioPlayed");
        });
        if (deps.canUseKey?.() === false) {
          turnOff();
          return;
        }
        if (mine !== gen) return;
        if (ticket !== skipSpeech || played.stopped) break;
        store.set((s) => ({
          ...s,
          lastSpoken: {
            text: item.text,
            chars: item.text.length,
            seconds: played.seconds,
            kind: item.kind,
          },
          spoken: s.spoken + 1,
        }));
      } catch (thrown) {
        if (deps.canUseKey?.() === false) {
          turnOff();
          return;
        }
        if (mine !== gen) return;
        if (ticket !== skipSpeech) break;
        // A failed reply keeps the mode on; the loop goes on listening.
        patch({
          error: `The answer could not be read aloud: ${toCallError(thrown).error.message}`,
        });
      }
    }
    if (mine !== gen) return;
    currentSpeech = null;
    busy = null;
    settle();
  };

  const fail = (message: string): void => {
    capture?.stop();
    capture = null;
    streamId = null;
    detector = null;
    busy = null;
    patch({ phase: "error", error: message, micPending: false, partial: "" });
  };

  const append = (id: string, pcm: Uint8Array): void => {
    chain = chain.then(async () => {
      if (streamId !== id) return;
      if (deps.canUseKey?.() === false) {
        turnOff();
        return;
      }
      try {
        const r = await speech({ op: "append", streamId: id, pcm: toBase64(pcm) });
        if (r.kind === "appended" && r.atLimit) void endTurn("limit");
      } catch (thrown) {
        if (deps.canUseKey?.() === false) {
          turnOff();
          return;
        }
        if (streamId === id) fail(toCallError(thrown).error.message);
      }
    });
  };

  /** Opens the microphone and a speech stream for the owner. */
  const listen = async (): Promise<void> => {
    if (deps.canUseKey?.() === false) {
      turnOff();
      return;
    }
    const mine = gen;
    busy = "listen";
    patch({ phase: "listening", partial: "", micPending: true, endedBy: null });
    let prepared;
    try {
      prepared = await deps.prepare();
    } catch (thrown) {
      if (deps.canUseKey?.() === false) {
        turnOff();
        return;
      }
      if (mine === gen) turnOff(micErrorText(thrown));
      return;
    }
    if (mine !== gen || deps.canUseKey?.() === false) {
      prepared.discard();
      if (deps.canUseKey?.() === false) turnOff();
      return;
    }
    let started: SpeechResult;
    try {
      started = await speech({
        op: "start",
        sampleRate: prepared.sampleRate,
        language: settings.store.get().language,
      });
    } catch (thrown) {
      prepared.discard();
      if (deps.canUseKey?.() === false) {
        turnOff();
        return;
      }
      if (mine === gen) fail(toCallError(thrown).error.message);
      return;
    }
    if (started.kind !== "started" || mine !== gen || deps.canUseKey?.() === false) {
      prepared.discard();
      if (started.kind === "started")
        void speech({ op: "cancel", streamId: started.streamId }).catch(() => undefined);
      if (deps.canUseKey?.() === false) turnOff();
      return;
    }
    const id = started.streamId;
    const rate = prepared.sampleRate;
    streamId = id;
    const s = settings.store.get();
    heardMs = 0;
    lastLoudAt = null;
    detector = createTurnEnd({ pauseMs: s.voiceChatPauseMs, words: started.mode === "realtime" });
    try {
      capture = await prepared.open({
        deviceId: s.micId,
        onChunk: (pcm) => {
          append(id, pcm);
          // The turn-end rule runs on audio time (ms of audio received), not the wall clock: chunks are audio.
          heardMs += (pcm.length / 2 / rate) * 1000;
          detector?.audio(pcmRms(pcm), (pcm.length / 2 / rate) * 1000, heardMs);
          if (pcmRms(pcm) >= (detector?.view().threshold ?? 0.01)) lastLoudAt = now();
          if (settings.store.get().voiceChatAutoEnd && detector?.ended(heardMs) === true)
            void endTurn("pause");
        },
        onEnded: () => {
          turnOff(
            "The microphone was lost, so voice chat stopped. Press Voice chat to start again.",
          );
        },
      });
    } catch (thrown) {
      void speech({ op: "cancel", streamId: id }).catch(() => undefined);
      if (deps.canUseKey?.() === false) {
        turnOff();
        return;
      }
      if (mine === gen) turnOff(micErrorText(thrown));
      return;
    }
    if (mine !== gen || streamId !== id || deps.canUseKey?.() === false) {
      capture.stop();
      capture = null;
      if (deps.canUseKey?.() === false) turnOff();
      return;
    }
    patch({ micPending: false });
  };

  /** Ends the user's turn now: the words go out as a voice turn. */
  const endTurn = async (by: TurnEndedBy): Promise<void> => {
    if (deps.canUseKey?.() === false) {
      turnOff();
      return;
    }
    const id = streamId;
    if (busy !== "listen" || id === null || store.get().micPending) return;
    const mine = gen;
    busy = "send";
    capture?.stop();
    capture = null;
    detector = null;
    patch({ phase: "sending", endedBy: by, latency: voiceLatency(lastLoudAt ?? now(), now()) });
    await chain;
    if (mine !== gen || streamId !== id) return;
    if (deps.canUseKey?.() === false) {
      turnOff();
      return;
    }
    let text: string;
    try {
      const r = await speech({ op: "stop", streamId: id });
      text = r.kind === "transcript" ? r.text.trim() : "";
    } catch (thrown) {
      if (deps.canUseKey?.() === false) {
        turnOff();
        return;
      }
      if (mine === gen) fail(toCallError(thrown).error.message);
      return;
    }
    if (mine !== gen) return;
    streamId = null;
    stamp("transcriptionDone");
    if (deps.canUseKey?.() === false) {
      turnOff();
      return;
    }
    if (text === "") {
      busy = null;
      patch({ note: "Nothing was heard.", partial: "" });
      settle();
      return;
    }
    patch({ heard: text, partial: "", note: null, error: null, phase: "thinking" });
    expect();
    stamp("turnSent");
    const sent = await composer.actions?.sendSpoken(store.get().owner ?? NEW_CHAT, text);
    if (mine !== gen) return;
    if (deps.canUseKey?.() === false) {
      turnOff();
      return;
    }
    busy = null;
    if (sent === undefined || !sent.ok) {
      expecting = null;
      patch({ phase: "error", error: sent?.message ?? "The app has no core." });
      return;
    }
    if (sent.started !== undefined) {
      patch({ owner: sent.started });
      known = new Set(endedTurns(entryOf(sent.started)).map((t) => t.id));
      observed = undefined;
    }
    settle();
  };

  /** After a send: think until the turn shows (or `EXPECT_MS` passed). */
  const expect = (): void => {
    expecting = now();
    if (expectTimer !== null) clearTimeout(expectTimer);
    expectTimer = setTimeout(() => {
      expectTimer = null;
      settle();
    }, EXPECT_MS);
  };

  /** The owner's entry last looked at (`observe` reads history only when it changed). */
  let observed: object | undefined;
  /** A removal check is scheduled (`settle`). */
  let goneCheck = false;
  /** When the owner was handed off to a new id (its snapshot may still be on the way), and whether to rebase. */
  let handoff: number | null = null;
  let rebase = false;
  const streamAnswer = (entry: ReturnType<typeof entryOf>, turnId: string, done = false): void => {
    if (mutedTurns.has(turnId)) return;
    const answers = turnAnswers(entry, turnId);
    for (const [index, answer] of answers.entries()) {
      const key = `${turnId}/${answer.id}`;
      let take = streams.get(key);
      if (take === undefined) {
        let budget = speechBudgets.get(turnId);
        if (budget === undefined) {
          budget = { chars: 0, truncated: false };
          speechBudgets.set(turnId, budget);
        }
        take = createVoiceReplyStream(budget);
        streams.set(key, take);
      }
      const chunks = take(answer.text, done || answer.done || index < answers.length - 1);
      for (const text of chunks) queue.push({ text, kind: "answer", turnId });
    }
    prefetch();
  };
  /** Notices finished turns and open prompts of the owner, queuing what to say. */
  const observe = (
    entry: ReturnType<typeof entryOf>,
    prompts: ReturnType<typeof openPrompts>,
  ): void => {
    if (entry === undefined || entry === observed) return;
    observed = entry;
    const running = runningAnswer(entry);
    if (running.trim() !== "") {
      stamp("firstModelToken");
      if (/[.!?](?:["')\]]*)(?:\s|$)/.test(running)) stamp("firstSentence");
    }
    const active = voiceTurnId(entry);
    const ended = endedTurns(entry);
    if (active !== null && !known.has(active) && !ended.some((turn) => turn.id === active))
      streamAnswer(entry, active);
    for (const turn of ended) {
      if (known.has(turn.id)) continue;
      known.add(turn.id);
      expecting = null;
      stamp("turnEnd");
      if (finalAnswer(entry, turn.id).trim() !== "") stamp("firstModelToken");
      if (/[.!?](?:["')\]]*)(?:\s|$)/.test(finalAnswer(entry, turn.id))) stamp("firstSentence");
      if (turn.how === "answer") streamAnswer(entry, turn.id, true);
      else {
        // A stopped/failed turn must not drain its prefetched answer after completion.
        queue = queue.filter((item) => item.turnId !== turn.id);
        if (currentSpeech?.turnId === turn.id) {
          skipSpeech += 1;
          player.stop();
        }
        mutedTurns.add(turn.id);
        if (turn.how === "failed") queue.push({ text: FAILED_SAID, kind: "failed" });
      }
      for (const key of streams.keys()) if (key.startsWith(`${turn.id}/`)) streams.delete(key);
      speechBudgets.delete(turn.id);
    }
    const newPrompts: Say[] = [];
    for (const prompt of prompts) {
      if (prompted.has(prompt.callbackId)) continue;
      prompted.add(prompt.callbackId);
      newPrompts.push({ text: prompt.text, kind: "prompt" });
    }
    queue.unshift(...newPrompts);
    prefetch();
  };

  /** Decides the next step when nothing is in flight. */
  function settle(ready = false): void {
    if (deps.canUseKey?.() === false) {
      turnOff();
      return;
    }
    const s = store.get();
    if (!s.on || s.owner === null) return;
    const entry = entryOf(s.owner);
    if (s.owner !== NEW_CHAT) {
      if (entry === undefined) {
        // The model drops a session before the core's listeners hear why: a hand-off (compaction or a clear inside
        // a turn) names the new id right after, so the check waits a moment; a removal turns voice chat off.
        if (handoff !== null && now() - handoff < HANDOFF_WAIT_MS) return;
        if (!core.store.get().index.loading && !goneCheck) {
          goneCheck = true;
          queueMicrotask(() => {
            goneCheck = false;
            const owner = store.get().owner;
            if (
              store.get().on &&
              owner !== null &&
              owner !== NEW_CHAT &&
              entryOf(owner) === undefined
            ) {
              if (handoff === null || now() - handoff >= HANDOFF_WAIT_MS) {
                turnOff("The session closed, so voice chat stopped.");
              }
            } else settle();
          });
        }
        return;
      }
      if (rebase) {
        // The continued session's earlier turns are history, not news: never spoken.
        rebase = false;
        for (const t of endedTurns(entry)) known.add(t.id);
      }
      if (sessionClosed(entry)) {
        turnOff("The session was archived, so voice chat stopped.");
        return;
      }
    }
    // A turn started while listening (typed with Enter, or a queued item): stop listening (the listen still opening
    // gives up: `gen`), the reply comes.
    if (busy === "listen" && sessionBusy(entry)) {
      gen += 1;
      halt();
      patch({ phase: "thinking", partial: "", micPending: false });
    }
    const prompts = entry === undefined || entry === observed ? [] : openPrompts(entry);
    observe(entry, prompts);
    if (busy !== null) return;
    if (queue.length > 0) {
      void speakAll();
      return;
    }
    // Cheap test first: the history is read only when a callback can be open.
    if (
      entry !== undefined &&
      entry.openCallbackIds.length + prompted.size > 0 &&
      openPrompts(entry).length > 0
    ) {
      patch({ phase: "waiting" });
      return;
    }
    // `expecting` stays until a turn ends (`observe`): between a queued item leaving the queue and its turn starting
    // the session looks idle for a moment (live run 2026-10-10: the microphone opened for 0.1 s in that gap).
    if (sessionBusy(entry)) {
      patch({ phase: "thinking" });
      return;
    }
    if (expecting !== null && now() - expecting < EXPECT_MS) {
      patch({ phase: "thinking" });
      return;
    }
    if (s.phase === "error") return;
    // Listen only once the session stayed idle for `LISTEN_SETTLE_MS` (a turn about to start shows up first).
    if (!ready) {
      if (listenTimer === null) {
        listenTimer = setTimeout(() => {
          listenTimer = null;
          settle(true);
        }, LISTEN_SETTLE_MS);
      }
      return;
    }
    void listen();
  }

  /** Turns voice chat on for `sessionId` (moving it from another session). Returns why not, or `null`. */
  const turnOn = (sessionId: string): string | null => {
    if (deps.canUseKey?.() === false) return null;
    if (deps.dictating())
      return "Finish the dictation first: voice chat and dictation share the microphone.";
    const s = store.get();
    if (s.on && s.owner === sessionId) return null;
    if (s.on && s.owner !== null) {
      turnOff(null);
    }
    gen += 1;
    deps.onStart();
    known = new Set(endedTurns(entryOf(sessionId)).map((t) => t.id));
    prompted = new Set();
    observed = undefined;
    store.set((x) => ({
      ...VOICE_CHAT_OFF,
      on: true,
      owner: sessionId,
      phase: "thinking",
      lastSpoken: x.lastSpoken,
      spoken: x.spoken,
    }));
    settle();
    return null;
  };

  core.store.subscribe(() => {
    if (store.get().on) settle();
  });
  core.listen((event: CoreEvent) => {
    // A session hand-off: the conversation continues under a new id; voice chat follows it.
    if (event.type === "change" && event.change.kind === "handedOff") {
      const { oldSessionId, newSessionId } = event.change;
      if (store.get().on && store.get().owner === oldSessionId) {
        handoff = now();
        rebase = true;
        observed = undefined;
        patch({ owner: newSessionId });
        // If the new id never shows, the removal check runs once the wait is over.
        setTimeout(() => {
          settle();
        }, HANDOFF_WAIT_MS + 10);
        settle();
      }
      return;
    }
    if (event.type !== "dictation" || event.streamId !== streamId) return;
    if (deps.canUseKey?.() === false) {
      turnOff();
      return;
    }
    const u = event.update;
    if (u.type === "partial") {
      if (u.text !== store.get().partial) detector?.words(heardMs);
      patch({ partial: u.text });
    } else if (u.type === "liveLost") {
      patch({ note: "Live text stopped; your words are sent when your turn ends." });
    } else {
      fail(u.reason);
    }
  });

  return {
    store,
    turnOn,
    turnOff,
    /** Ends the user's turn now (the route; the button uses `press`). */
    endTurn: (): Promise<void> => endTurn("button"),
    /**
     * The Voice chat button of `sessionId`: off or another owner → on here; listening → end my turn; speaking → stop
     * the reply and listen; error → try again; sending, thinking, waiting → nothing (the card or the turn decides).
     */
    press(sessionId: string): string | null {
      const s = store.get();
      if (!s.on || s.owner !== sessionId) return turnOn(sessionId);
      if (s.phase === "listening") void endTurn("button");
      else if (s.phase === "speaking") {
        const active = voiceTurnId(entryOf(s.owner));
        if (active !== null) {
          mutedTurns.add(active);
          void composer.actions?.stop(sessionId);
        }
        queue = [];
        skipSpeech += 1;
        player.stop();
      } else if (s.phase === "error") {
        patch({ error: null, phase: "thinking" });
        settle();
      }
      return null;
    },
    /** True when `sessionId` is in voice chat (its turns are voice turns; narration leaves it alone). */
    owns(sessionId: string): boolean {
      const s = store.get();
      return s.on && s.owner === sessionId;
    },
    /** True while voice chat holds or may soon reopen the microphone (dictation waits). */
    active(): boolean {
      return store.get().on;
    },
  };
}

/** The voice chat controller. */
export type VoiceChat = ReturnType<typeof createVoiceChat>;
