/**
 * Answering the agent's question card by voice in voice chat. When voice chat is about to read a question card of its
 * chat, the card's questions are read one at a time with numbered options and a short hint; after each reading the
 * microphone opens for the answer; the answer (`features/voice/voiceAnswer`) is sent through the same answer path as
 * a click on the card, and a short sentence says what was sent.
 * Depends on: coreStore, voiceAnswer, transcript callbacks, vocabulary. Called by: state/voiceChat.
 * Invariants:
 * - Works wherever the person looks (another chat, Settings, another app): voice chat's chat owns it, not the screen.
 * - Questions grant no permission: this never answers an approval, and approvals keep their own voice rules.
 * - Never a guess: an unclear answer is asked again once, then the card is left for the screen.
 * - The card answered or closed meanwhile, or the reading turned off: the voice answer ends (`reconcile`).
 */
import { type Heard, hearAnswer } from "../features/voice/voiceAnswer";
import { MAX_SPOKEN_CHARS } from "../features/voice/voiceChatText";
import {
  type Picked,
  type Question,
  questionCancelOutput,
  questionOutput,
  questionsOf,
} from "../features/transcript/model/callbacks";
import { type Obj, arr, obj, str } from "../features/transcript/model/entries";
import type { JsonValue, SpeechRequest, SpeechResult } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import { TEXT, wording } from "../vocabulary";
import type { CoreController } from "./coreStore";
import { createStore, type Store } from "./createStore";
import { VOICE_ANSWER_ENDED, type VoiceQuestionReply, type VoiceQuestionState } from "./voiceTypes";

export { VOICE_ANSWER_ENDED, type VoiceQuestionReply, type VoiceQuestionState };

/**
 * The sentence telling the person what was heard (`""`: nothing to add). Also used for a missed permission answer
 * (`voicePrompt`).
 */
export function heardSentence(text: string, free: boolean): string {
  if (text !== "")
    return wording(TEXT.voiceQuestionHeard, { heard: text.replace(/\s+/g, " ").slice(0, 80) });
  return free ? "" : TEXT.voiceQuestionHeardNothing;
}

/** How long the microphone waits for an answer before the card is left for the screen. */
const SILENCE_MS = 60_000;
const TOTAL_ANSWER_MS = 180_000;

/**
 * The card's questions when every one can take a spoken answer, else `null`. A card shown here for another chat (a
 * subagent's, `vwSourceSession`) is answered in that chat: it stays for the screen.
 */
function voiceQuestions(entry: Obj | null): Question[] | null {
  if (entry === null || obj(entry["detail"])?.["kind"] !== "user_input") return null;
  if (typeof entry["vwSourceSession"] === "string") return null;
  const { questions } = questionsOf(entry);
  if (questions.length === 0) return null;
  return questions.every((q) => q.options.length > 0 || q.allowOther) ? questions : null;
}

/** The words read for question `index`: the question, its numbered options and what to say. */
export function questionSaid(questions: readonly Question[], index: number): string {
  const q = questions[index];
  if (q === undefined) return "";
  const question = q.question.replace(/\s+/g, " ").trim().slice(0, 240);
  const head =
    questions.length === 1
      ? wording(TEXT.voiceQuestionAsk, { question })
      : index === 0
        ? wording(TEXT.voiceQuestionMany, { count: String(questions.length), question })
        : wording(TEXT.voiceQuestionNext, { n: String(index + 1), question });
  const options = q.options.map(
    (o, i) => `${String(i + 1)}: ${o.label.replace(/\s+/g, " ").trim().slice(0, 90)}.`,
  );
  const hint =
    q.options.length === 0
      ? TEXT.voiceQuestionFree
      : `${q.multiSelect ? TEXT.voiceQuestionChooseMany : TEXT.voiceQuestionChoose}${
          q.allowOther ? ` ${TEXT.voiceQuestionOwnWords}` : ""
        }`;
  return [head, ...options, hint].join(" ").slice(0, MAX_SPOKEN_CHARS);
}

/** One answer as said back: the chosen options, then the own words. */
const told = (p: Picked) => [...p.labels, p.other.trim()].filter((t) => t !== "").join(", ");

/** Creates the voice answer for question cards. */
export function createVoiceQuestion(deps: {
  core: CoreController;
  speech: (request: SpeechRequest) => Promise<SpeechResult>;
  /** Voice chat's chat while voice chat is on. */
  owner: () => string | null;
  /** Questions are read aloud and the key works. */
  enabled: () => boolean;
  /** Sends the card's answer exactly as the card's own buttons do. */
  answer: (sessionId: string, callbackId: string, output: JsonValue) => Promise<boolean>;
  /** The voice answer ended without an answer (card closed, silence): voice chat stops listening for it. */
  interrupted: (message?: string) => void;
  store?: Store<VoiceQuestionState | null> | undefined;
}) {
  const store = deps.store ?? createStore<VoiceQuestionState | null>(null);
  let questions: Question[] = [];
  let picks: Picked[] = [];
  let misses = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let capTimer: ReturnType<typeof setTimeout> | null = null;
  let firstListenAt: number | null = null;
  const historyEntry = (sessionId: string, callbackId: string) => {
    const session = deps.core.store.get().sessions[sessionId];
    const found = arr(session?.state?.["history"]).find((raw) => {
      const e = obj(raw);
      return e !== null && (str(e["callbackId"]) ?? str(e["id"])) === callbackId;
    });
    return { session, entry: obj(found ?? null) };
  };
  const open = (sessionId: string, callbackId: string): boolean => {
    const { session, entry } = historyEntry(sessionId, callbackId);
    if (session === undefined || entry === null) return false;
    const status = obj(entry["state"])?.["status"];
    return session.openCallbackIds.includes(callbackId) || status === "open";
  };
  const cancel = (): void => {
    if (timer !== null) clearTimeout(timer);
    if (capTimer !== null) clearTimeout(capTimer);
    timer = null;
    capTimer = null;
    firstListenAt = null;
    store.set(null);
  };
  const expired = (): void => {
    cancel();
    deps.interrupted(VOICE_ANSWER_ENDED);
  };
  /** Sends the card's answer; the result is what voice chat says. */
  const send = async (current: VoiceQuestionState, output: JsonValue | null, said: string) => {
    cancel();
    const ok =
      output !== null && (await deps.answer(current.sessionId, current.callbackId, output));
    return { again: null, said: ok ? said : TEXT.voiceQuestionFailed };
  };
  const heard = async (
    current: VoiceQuestionState,
    h: Heard,
    text: string,
    failure: string,
  ): Promise<VoiceQuestionReply> => {
    const question = questions[current.index];
    if (question === undefined || h.kind === "unclear") {
      misses += 1;
      // The person may not be looking: what was heard (or that nothing was) is said, shown and kept in the local log.
      const free = question !== undefined && question.options.length === 0;
      const lead = heardSentence(text, free);
      void deps
        .speech({
          op: "heard",
          text: text !== "" ? text : `no words${failure === "" ? "" : `: ${failure}`}`,
        })
        .catch(() => undefined);
      if (misses >= 2) {
        cancel();
        return { again: null, said: `${lead} ${TEXT.voiceQuestionCard}`.trim(), heard: lead };
      }
      store.set({ ...current, listening: false });
      const again = free ? TEXT.voiceQuestionAgainFree : TEXT.voiceQuestionAgain;
      return { again: `${lead} ${again}`.trim(), said: null, heard: lead };
    }
    if (h.kind === "repeat") {
      store.set({ ...current, listening: false });
      return { again: questionSaid(questions, current.index), said: null };
    }
    if (h.kind === "skip") return send(current, questionCancelOutput(), TEXT.voiceQuestionSkipped);
    picks[current.index] = h.picked;
    misses = 0;
    const next = current.index + 1;
    if (next < questions.length) {
      store.set({ ...current, index: next, listening: false });
      return { again: `${told(h.picked)}. ${questionSaid(questions, next)}`, said: null };
    }
    const answer = picks.map(told).join("; ");
    return send(
      current,
      questionOutput(questions, picks),
      wording(TEXT.voiceQuestionAnswered, { answer }),
    );
  };
  return {
    store,
    cancel,
    /**
     * Called before a card's prompt is read: a question card of voice chat's chat gets its first question's words
     * (and is answered by voice); any other card keeps `text`. One card at a time.
     */
    prepare(callbackId: string, text: string): string {
      const owner = deps.owner();
      if (owner === null || !deps.enabled() || store.get() !== null) return text;
      if (!open(owner, callbackId)) return text;
      const found = voiceQuestions(historyEntry(owner, callbackId).entry);
      if (found === null) return text;
      questions = found;
      picks = found.map(() => ({ labels: [], other: "" }));
      misses = 0;
      firstListenAt = null;
      store.set({ sessionId: owner, callbackId, index: 0, ready: false, listening: false });
      return questionSaid(found, 0);
    },
    /** The reading of `callbackId` played to its end: the next listening is for its answer. */
    played(callbackId: string): void {
      const current = store.get();
      if (current?.callbackId === callbackId && deps.enabled())
        store.set({ ...current, ready: true });
    },
    ready: () => store.get()?.ready === true && deps.enabled(),
    /** The reading stopped before its end and is not queued again (`queued`): the card is left for the screen. */
    unread(queued: (callbackId: string) => boolean): void {
      const current = store.get();
      if (current !== null && !current.ready && !current.listening && !queued(current.callbackId))
        cancel();
    },
    /** The microphone opened for the answer: re-arm for a minute, within one three-minute card deadline. */
    listening(): void {
      const current = store.get();
      if (current === null) return;
      const now = Date.now();
      if (firstListenAt === null) {
        firstListenAt = now;
        // This timer is independent of wall-clock changes and is never re-armed.
        capTimer = setTimeout(expired, TOTAL_ANSWER_MS);
      }
      const remaining = firstListenAt + TOTAL_ANSWER_MS - now;
      if (remaining <= 0) {
        expired();
        return;
      }
      store.set({ ...current, ready: false, listening: true });
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(expired, Math.min(SILENCE_MS, remaining));
    },
    /** The person's turn ended: transcribes `streamId` and acts on the answer. */
    async hear(streamId: string): Promise<VoiceQuestionReply & { note: string }> {
      const current = store.get();
      if (timer !== null) clearTimeout(timer);
      timer = null;
      // A failed transcription counts as nothing heard.
      let failure = "";
      const text = await deps.speech({ op: "stop", streamId }).then(
        (r) => (r.kind === "transcript" ? r.text.trim() : ""),
        (thrown: unknown) => {
          failure = toCallError(thrown).error.message;
          return "";
        },
      );
      if (current === null || store.get()?.callbackId !== current.callbackId)
        return { again: null, said: null, note: "" };
      const question = questions[current.index];
      const reply = await heard(
        current,
        question === undefined ? { kind: "unclear" } : hearAnswer(question, text),
        text,
        failure,
      );
      return { ...reply, note: reply.heard ?? reply.said ?? "" };
    },
    /** Ends the voice answer when its card is no longer open in voice chat's chat or the reading turned off. */
    reconcile(): void {
      const current = store.get();
      if (current === null) return;
      if (
        deps.enabled() &&
        deps.owner() === current.sessionId &&
        open(current.sessionId, current.callbackId)
      )
        return;
      cancel();
      deps.interrupted();
    },
  };
}

/** The question voice answer controller. */
export type VoiceQuestion = ReturnType<typeof createVoiceQuestion>;
