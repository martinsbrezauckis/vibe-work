/**
 * Voice chat owns one microphone and speech stream, sends spoken turns, reads replies and cards, then listens again.
 * A pause carries later words through thinking until playback starts; the microphone closes before playback.
 * The strip says Listening after sound reaches the speech stream. Async work checks `gen` before changing state.
 * Depends on voice capture, speech, composer, core store and voice settings. Called by state/voice.
 */
import { quietPlayback } from "../features/voice/playbackGate";
import { createVoicePrompt } from "./voicePrompt";
import { createVoiceQuestion } from "./voiceQuestion";
import { CARRY_OVERFLOW, createCarry } from "./voiceCarry";
import { createSoundWait } from "./voiceSoundWait";
import type { CoreEvent, SpeechRequest, SpeechResult } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import { micErrorText } from "../features/voice/pcmCapture";
import { type TurnEnd, createTurnEnd, pcmRms } from "../features/voice/turnEnd";
import { FAILED_SAID } from "../features/voice/voiceChatText";
import { type VoiceLatency, voiceLatency } from "../features/voice/voiceLatency";
import { createTurnReplies } from "../features/voice/voiceReplyStream";
import { createStore } from "./createStore";
import { toBase64 } from "./dictation";
import { NEW_CHAT } from "./newChatKey";
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
import {
  VOICE_CHAT_OFF,
  type VoiceChatState,
  type VoiceChatDeps,
  type VoiceChatUtterance as Say,
  type TurnEndedBy,
  EXPECT_MS,
  LISTEN_SETTLE_MS,
  SENTENCE_GAP_MS,
} from "./voiceChatState";
export {
  VOICE_CHAT_OFF,
  SENTENCE_GAP_MS,
  type VoiceChatState,
  type VoiceChatPhase,
  type TurnEndedBy,
  type SpokenKind,
} from "./voiceChatState";
/** Creates the one-owner voice chat controller. */
export function createVoiceChat(deps: VoiceChatDeps) {
  const { core, composer, settings, toasts, player } = deps;
  const now = deps.now ?? (() => performance.timeOrigin + performance.now());
  const store = deps.store ?? createStore<VoiceChatState>(VOICE_CHAT_OFF);
  let gen = 0;
  let busy: "listen" | "send" | "speak" | null = null;
  let capture: { stop(): void } | null = null;
  let streamId: string | null = null;
  let chain: Promise<unknown> = Promise.resolve();
  let detector: TurnEnd | null = null;
  const carry = createCarry();
  let carrying = false;
  let readingPaused = false;
  let heardMs = 0;
  let known = new Set<string>();
  let prompted = new Set<string>();
  let expecting: number | null = null;
  let expectTimer: ReturnType<typeof setTimeout> | null = null;
  let listenTimer: ReturnType<typeof setTimeout> | null = null;
  const soundWait = createSoundWait();
  let queue: Say[] = [];
  let currentSpeech: Say | null = null;
  const replies = createTurnReplies();
  const mutedTurns = new Set<string>();
  let skipSpeech = 0;
  let lastLoudAt: number | null = null;
  let promptListening = false;
  let questionListening = false;
  let restoreOutput: (() => void) | null = null;
  let lastSentence: { turnId: string | undefined; at: number } | null = null;
  const entryOf = (id: string | null) => (id === null ? undefined : core.store.get().sessions[id]);
  const speech = async (request: SpeechRequest): Promise<SpeechResult> => {
    const r = await core.run({ type: "speech", request });
    if (r.type !== "speech") throw new Error("unexpected answer to a speech request");
    return r.result;
  };
  const patch = (p: Partial<VoiceChatState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  const interrupted = (message?: string): void => {
    gen++;
    question.cancel();
    const waiting = queue;
    halt();
    queue = waiting.filter(
      (item) =>
        item.callbackId === undefined ||
        core.store.get().asks[`callback:${item.callbackId}`] !== undefined,
    );
    if (message !== undefined) queue.push({ text: message, kind: "prompt" });
    patch({ phase: "waiting", partial: "", micPending: false, note: message ?? null });
    settle();
  };
  const owner = () => (store.get().on ? store.get().owner : null);
  const prompt = createVoicePrompt({
    core,
    accessibility: deps.accessibility,
    speech,
    owner,
    blocked: () => deps.promptBlocked?.() === true || deps.canUseKey?.() === false,
    language: () => settings.store.get().language,
    interrupted,
    ...(deps.promptStore === undefined ? {} : { store: deps.promptStore }),
  });
  const question = createVoiceQuestion({
    core,
    speech,
    owner,
    enabled: () =>
      deps.accessibility?.store.get().readPrompts !== "never" && deps.canUseKey?.() !== false,
    answer:
      deps.answer ??
      ((sessionId, callbackId, output) =>
        core.run({ type: "answerCallback", sessionId, callbackId, output }).then(
          () => true,
          () => false,
        )),
    interrupted,
    ...(deps.questionStore === undefined ? {} : { store: deps.questionStore }),
  });
  const stamp = (stage: keyof VoiceLatency): void => {
    const t = store.get().latency;
    if (t !== null && t[stage] === null) patch({ latency: { ...t, [stage]: now() } });
  };
  const closeCapture = (): void => {
    capture?.stop();
    capture = null;
  };
  const halt = (): void => {
    promptListening = false;
    questionListening = false;
    carry.take();
    carrying = false;
    readingPaused = false;
    closeCapture();
    restoreOutput?.();
    restoreOutput = null;
    const id = streamId;
    streamId = null;
    detector = null;
    if (id !== null) void speech({ op: "cancel", streamId: id }).catch(() => undefined);
    if (busy === "speak") player.stop();
    queue = [];
    currentSpeech = null;
    replies.clear();
    mutedTurns.clear();
    skipSpeech += 1;
    busy = null;
    expecting = null;
    if (expectTimer !== null) clearTimeout(expectTimer);
    expectTimer = null;
    if (listenTimer !== null) clearTimeout(listenTimer);
    listenTimer = null;
    soundWait.clear();
  };
  const silence = (): void => {
    prompt.cancel();
    question.cancel();
    queue = [];
    skipSpeech += 1;
    player.stop();
  };
  /**
   * A card answered or closed while its prompt is read or waits to be read: that prompt is not read any further
   * (the person already chose; reading the options on would talk over what comes next).
   */
  const dropClosedPrompts = (entry: ReturnType<typeof entryOf>): void => {
    const reading = busy === "speak" ? currentSpeech?.callbackId : undefined;
    if (reading === undefined && !queue.some((item) => item.callbackId !== undefined)) return;
    const open = new Set(openPrompts(entry).map((p) => p.callbackId));
    queue = queue.filter((item) => item.callbackId === undefined || open.has(item.callbackId));
    if (reading !== undefined && !open.has(reading)) {
      skipSpeech += 1;
      player.stop();
    }
  };
  const prepareSpeech = (item: Say): Promise<SpeechResult> => {
    if (item.ready !== undefined) return item.ready;
    if (item.kind === "answer") stamp("firstTtsRequest");
    item.ready = prompt.speak(item.callbackId, item.text, settings.store.get().voice);
    void item.ready.catch(() => undefined);
    return item.ready;
  };
  const prefetch = (): void => {
    if (busy === "speak" && currentSpeech !== null && queue[0]?.kind === "answer")
      void prepareSpeech(queue[0]);
  };
  const turnOff = (reason: string | null = null): void => {
    if (!store.get().on) return;
    gen += 1;
    prompt.end();
    question.cancel();
    halt();
    store.set((s) => ({
      ...VOICE_CHAT_OFF,
      lastSpoken: s.lastSpoken,
      spoken: s.spoken,
      heard: s.heard,
      latency: s.latency,
      note: reason,
    }));
    if (reason !== null) toasts.push({ kind: "warn", bellKind: "failure", text: reason });
  };
  const keyLost = (): boolean => {
    if (deps.canUseKey?.() !== false) return false;
    turnOff();
    return true;
  };

  const speakAll = async (): Promise<void> => {
    if (keyLost()) return;
    const mine = gen;
    busy = "speak";
    while (queue.length > 0 && mine === gen) {
      if (keyLost()) return;
      const item = queue.shift();
      if (item === undefined) break;
      currentSpeech = item;
      const ticket = skipSpeech;
      patch({
        phase: "speaking",
        ...(item.kind === "answer" && store.get().note !== CARRY_OVERFLOW ? { note: null } : {}),
      });
      try {
        if (item.callbackId !== undefined && !item.repeat) {
          const asked = question.prepare(item.callbackId, item.text);
          item.text =
            asked !== item.text ? asked : await prompt.prepare(item.callbackId, item.text);
        }
        if (mine !== gen || ticket !== skipSpeech) break;
        const r = await prepareSpeech(item);
        if (keyLost()) return;
        if (mine !== gen || ticket !== skipSpeech || r.kind !== "speech") break;
        prefetch();
        const wait =
          item.kind === "answer" && lastSentence !== null && lastSentence.turnId === item.turnId
            ? SENTENCE_GAP_MS - (now() - lastSentence.at)
            : 0;
        if (wait > 0) {
          await new Promise((resolve) => setTimeout(resolve, wait));
          if (mine !== gen || ticket !== skipSpeech) break;
        }
        if (capture !== null) {
          readingPaused = true;
          if (carrying) carry.markCutoff(settings.store.get().voiceChatPauseMs);
          carrying = false;
          closeCapture();
          patch({ carryOpen: false });
        }
        const played = await player.play(
          Uint8Array.from(atob(r.audio), (c) => c.charCodeAt(0)),
          () => {
            if (mine === gen && ticket === skipSpeech && item.kind === "answer")
              stamp("firstAudioPlayed");
          },
        );
        if (keyLost()) return;
        if (mine !== gen) return;
        lastSentence = item.kind === "answer" ? { turnId: item.turnId, at: now() } : null;
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
        if (item.callbackId !== undefined) {
          prompt.played(item.callbackId);
          question.played(item.callbackId);
          if (prompt.ready() || question.ready()) break;
        }
      } catch (thrown) {
        if (item.callbackId !== undefined) {
          prompt.cancel();
          question.cancel();
        }
        if (keyLost()) return;
        if (mine !== gen) return;
        if (ticket !== skipSpeech) break;
        patch({
          error: `The answer could not be read aloud: ${toCallError(thrown).error.message}`,
        });
      }
    }
    if (mine !== gen) return;
    question.unread((id) => queue.some((item) => item.callbackId === id));
    currentSpeech = null;
    busy = null;
    settle();
  };

  const fail = (message: string): void => {
    prompt.cancel();
    question.cancel();
    closeCapture();
    carrying = false;
    restoreOutput?.();
    restoreOutput = null;
    streamId = null;
    detector = null;
    busy = null;
    patch({ phase: "error", error: message, micPending: false, partial: "", carryOpen: false });
  };
  const append = (id: string, pcm: Uint8Array, delivered?: () => void): void => {
    chain = chain.then(async () => {
      if (streamId !== id) return;
      if (keyLost()) return;
      try {
        const r = await speech({ op: "append", streamId: id, pcm: toBase64(pcm) });
        if (r.kind === "appended") {
          delivered?.();
          if (r.atLimit) void endTurn("limit");
        }
      } catch (thrown) {
        if (keyLost()) return;
        if (streamId === id && !prompt.endOnError(thrown, promptListening && !questionListening))
          fail(toCallError(thrown).error.message);
      }
    });
  };

  const listen = async (): Promise<void> => {
    if (keyLost()) return;
    const mine = gen;
    const resumed = readingPaused;
    readingPaused = false;
    carrying = false;
    closeCapture();
    const permission = prompt.ready();
    questionListening = !permission && question.ready();
    promptListening = permission || questionListening;
    // Only a permission answer mutes the app's sounds (a recording must not say "allow").
    const restore = permission ? quietPlayback() : null;
    restoreOutput = restore;
    busy = "listen";
    patch({
      phase: "listening",
      partial: "",
      micPending: true,
      awaitingSound: true,
      endedBy: null,
      carryOpen: false,
      resumed,
    });
    let prepared;
    try {
      prepared = await deps.prepare();
    } catch (thrown) {
      if (keyLost()) return;
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
      started = permission
        ? await prompt.start(prepared.sampleRate, settings.store.get().language)
        : await speech({
            op: "start",
            sampleRate: prepared.sampleRate,
            language: settings.store.get().language,
          });
    } catch (thrown) {
      prepared.discard();
      if (keyLost()) return;
      if (mine === gen && !prompt.endOnError(thrown, permission))
        fail(toCallError(thrown).error.message);
      return;
    }
    if (started.kind !== "started" || mine !== gen || deps.canUseKey?.() === false) {
      prepared.discard();
      if (started.kind === "started")
        void speech({ op: "cancel", streamId: started.streamId }).catch(() => undefined);
      restore?.();
      if (restoreOutput === restore) restoreOutput = null;
      if (deps.canUseKey?.() === false) turnOff();
      return;
    }
    const id = started.streamId;
    const rate = prepared.sampleRate;
    streamId = id;
    if (questionListening) question.listening();
    const s = settings.store.get();
    heardMs = 0;
    lastLoudAt = null;
    detector = createTurnEnd({ pauseMs: s.voiceChatPauseMs, words: started.mode === "realtime" });
    const feed = (pcm: Uint8Array): void => {
      append(
        id,
        pcm,
        soundWait.heard(id, pcm)
          ? () => {
              if (busy === "listen" && streamId === id && store.get().awaitingSound) {
                soundWait.clear();
                patch({ awaitingSound: false });
              }
            }
          : undefined,
      );
      heardMs += (pcm.length / 2 / rate) * 1000;
      detector?.audio(pcmRms(pcm), (pcm.length / 2 / rate) * 1000, heardMs);
      if (pcmRms(pcm) >= (detector?.view().threshold ?? 0.01)) lastLoudAt = now();
      if (settings.store.get().voiceChatAutoEnd && detector?.ended(heardMs) === true)
        void endTurn("pause");
    };
    // Carried words come first, in an ordinary stretch only: a card's answer never hears audio from before it.
    const hadCarry = !promptListening && carry.hasSpeech();
    if (hadCarry) detector.audio(0, 0, 0);
    if (!promptListening) for (const pcm of carry.take()) feed(pcm);
    try {
      capture = await prepared.open({
        deviceId: s.micId,
        onChunk: (pcm) => {
          if (carrying) {
            if (carry.keep(pcm, pcmRms(pcm), rate)) {
              patch({ note: CARRY_OVERFLOW });
              toasts.push({ kind: "warn", bellKind: "failure", text: CARRY_OVERFLOW });
            }
          } else if (busy === "listen" && streamId === id) feed(pcm);
        },
        onEnded: () => {
          turnOff(
            "The microphone was lost, so voice chat stopped. Press Voice chat to start again.",
          );
        },
      });
    } catch (thrown) {
      void speech({ op: "cancel", streamId: id }).catch(() => undefined);
      if (keyLost()) return;
      if (mine === gen) turnOff(micErrorText(thrown));
      return;
    }
    if (mine !== gen || streamId !== id || deps.canUseKey?.() === false) {
      closeCapture();
      restore?.();
      if (restoreOutput === restore) restoreOutput = null;
      if (deps.canUseKey?.() === false) turnOff();
      return;
    }
    patch({ micPending: false });
    if (hadCarry && s.voiceChatAutoEnd && detector.ended(heardMs)) {
      void endTurn("pause");
      return;
    }
    soundWait.arm(
      id,
      () => mine === gen && busy === "listen" && streamId === id && store.get().awaitingSound,
      turnOff,
    );
  };
  const endTurn = async (by: TurnEndedBy): Promise<void> => {
    if (keyLost()) return;
    const id = streamId;
    if (busy !== "listen" || id === null || store.get().micPending) return;
    const mine = gen;
    busy = "send";
    // A pause ended an ordinary turn: collect the person's next words until reply playback starts.
    carrying = by === "pause" && !promptListening;
    if (carrying) carry.arm(detector?.view().threshold ?? 0.01);
    else closeCapture();
    restoreOutput?.();
    restoreOutput = null;
    detector = null;
    patch({
      phase: "sending",
      endedBy: by,
      carryOpen: carrying,
      resumed: false,
      latency: voiceLatency(lastLoudAt ?? now(), now()),
    });
    await chain;
    if (mine !== gen || streamId !== id) return;
    if (keyLost()) return;
    if (promptListening) {
      const callbackId = (questionListening ? question : prompt).store.get()?.callbackId;
      const answered: { again: string | null; note: string; said?: string | null } =
        questionListening ? await question.hear(id) : await prompt.answer(id);
      if (mine !== gen) return;
      promptListening = false;
      questionListening = false;
      streamId = null;
      busy = null;
      patch({ partial: "", note: answered.note, phase: "waiting" });
      if (answered.again !== null && callbackId !== undefined)
        queue.unshift({ text: answered.again, kind: "prompt", callbackId, repeat: true });
      if (typeof answered.said === "string") queue.unshift({ text: answered.said, kind: "prompt" });
      settle();
      return;
    }
    let text: string;
    try {
      const r = await speech({ op: "stop", streamId: id });
      text = r.kind === "transcript" ? r.text.trim() : "";
    } catch (thrown) {
      if (keyLost()) return;
      if (mine === gen) fail(toCallError(thrown).error.message);
      return;
    }
    if (mine !== gen) return;
    streamId = null;
    stamp("transcriptionDone");
    if (keyLost()) return;
    if (text === "") {
      carrying = false;
      closeCapture();
      busy = null;
      patch({
        note:
          store.get().note === "Vibe Work could not reach the proxy."
            ? store.get().note
            : "Nothing was heard.",
        partial: "",
        carryOpen: false,
      });
      settle();
      return;
    }
    patch({
      heard: text,
      partial: "",
      note:
        store.get().note === CARRY_OVERFLOW ||
        store.get().note === "Vibe Work could not reach the proxy."
          ? store.get().note
          : null,
      error: null,
      phase: "thinking",
    });
    expect();
    stamp("turnSent");
    const sent = await composer.actions?.sendSpoken(store.get().owner ?? NEW_CHAT, text);
    if (mine !== gen) return;
    if (keyLost()) return;
    if (!carrying) closeCapture();
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
  const expect = (): void => {
    expecting = now();
    if (expectTimer !== null) clearTimeout(expectTimer);
    expectTimer = setTimeout(() => {
      expectTimer = null;
      settle();
    }, EXPECT_MS);
  };

  let observed: object | undefined;
  let goneCheck = false;
  const streamAnswer = (entry: ReturnType<typeof entryOf>, turnId: string, done = false): void => {
    if (mutedTurns.has(turnId)) return;
    const answers = turnAnswers(entry, turnId);
    for (const [index, answer] of answers.entries()) {
      const last = done || answer.done || index < answers.length - 1;
      for (const text of replies.take(turnId, answer.id, answer.text, last))
        queue.push({ text, kind: "answer", turnId });
    }
    prefetch();
  };
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
        queue = queue.filter((item) => item.turnId !== turn.id);
        if (currentSpeech?.turnId === turn.id) {
          skipSpeech += 1;
          player.stop();
        }
        mutedTurns.add(turn.id);
        if (turn.how === "failed") queue.push({ text: FAILED_SAID, kind: "failed" });
      }
      replies.forget(turn.id);
    }
    const newPrompts: Say[] = [];
    for (const prompt of prompts) {
      if (prompted.has(prompt.callbackId)) continue;
      prompted.add(prompt.callbackId);
      if (deps.accessibility?.store.get().readPrompts !== "never")
        newPrompts.push({ text: prompt.text, kind: "prompt", callbackId: prompt.callbackId });
    }
    queue.unshift(...newPrompts);
    prefetch();
  };

  function settle(ready = false): void {
    if (keyLost()) return;
    const s = store.get();
    if (!s.on || s.owner === null) return;
    const entry = entryOf(s.owner);
    if (s.owner !== NEW_CHAT) {
      if (entry === undefined) {
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
              turnOff("The chat closed, so voice chat stopped.");
            } else settle();
          });
        }
        return;
      }
      if (sessionClosed(entry)) {
        turnOff("The chat was archived, so voice chat stopped.");
        return;
      }
    }
    if (busy === "listen" && !promptListening && sessionBusy(entry)) {
      gen += 1;
      halt();
      patch({ phase: "thinking", partial: "", micPending: false });
    }
    dropClosedPrompts(entry);
    const prompts =
      entry === undefined || entry === observed
        ? []
        : openPrompts(entry, Object.values(core.store.get().asks));
    observe(entry, prompts);
    if (busy !== null) return;
    if (prompt.ready() || question.ready()) {
      void listen();
      return;
    }
    const cutoff = queue.length === 0 && !sessionBusy(entry) ? carry.takeCutoffNotice() : null;
    if (cutoff !== null) {
      queue.push({ text: cutoff, kind: "prompt" });
      patch({ note: cutoff });
    }
    if (queue.length > 0) {
      void speakAll();
      return;
    }
    if (
      entry !== undefined &&
      entry.openCallbackIds.length + prompted.size > 0 &&
      openPrompts(entry).length > 0
    ) {
      patch({ phase: "waiting" });
      return;
    }
    if (sessionBusy(entry) || (expecting !== null && now() - expecting < EXPECT_MS)) {
      patch({ phase: "thinking" });
      return;
    }
    if (s.phase === "error") return;
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

  const reconcileAll = (): void => {
    prompt.reconcile();
    question.reconcile();
  };
  deps.accessibility?.store.subscribe(reconcileAll);
  core.store.subscribe(() => {
    reconcileAll();
    if (store.get().on) settle();
  });
  let sendingBefore = composer.store.get().sending;
  composer.store.subscribe(() => {
    const sending = composer.store.get().sending;
    const before = sendingBefore;
    sendingBefore = sending;
    const s = store.get();
    if (!s.on || s.owner === null || busy !== "speak") return;
    if (sending[s.owner] !== true || before[s.owner] === true) return;
    const active = voiceTurnId(entryOf(s.owner));
    for (const item of [currentSpeech, ...queue])
      if (item?.turnId !== undefined && item.turnId !== active) mutedTurns.add(item.turnId);
    silence();
  });
  core.listen((event: CoreEvent) => {
    if (event.type !== "dictation" || event.streamId !== streamId) return;
    if (keyLost()) return;
    const u = event.update;
    if (u.type === "partial") {
      if (u.text !== store.get().partial) detector?.words(heardMs);
      patch({ partial: u.text });
    } else if (u.type === "liveLost") {
      patch({
        note:
          u.message === "Vibe Work could not reach the proxy."
            ? u.message
            : "Live text stopped; your words are sent when your turn ends.",
      });
    } else {
      fail(u.reason);
    }
  });
  return {
    store,
    prompt,
    question,
    turnOn,
    turnOff,
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
        const active =
          currentSpeech?.callbackId === undefined ? voiceTurnId(entryOf(s.owner)) : null;
        if (active !== null) {
          mutedTurns.add(active);
          void composer.actions?.stop(sessionId);
        }
        silence();
      } else if (s.phase === "error") {
        patch({ error: null, phase: "thinking" });
        settle();
      }
      return null;
    },
    owns: (sessionId: string): boolean => store.get().on && store.get().owner === sessionId,
    active: (): boolean => store.get().on,
  };
}
