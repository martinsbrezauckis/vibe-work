/**
 * The prompts waiting on a card or in a confirmation window of its own (an agent's Git guard, vault, saved login,
 * tabs) and the way to them: "Go to the waiting prompt" (Ctrl/Cmd+Shift+A, the palette, the composer's hint) moves
 * the focus to the oldest waiting prompt of the open session, else the oldest anywhere (opening its session); pressed
 * again while several wait there, it goes to the next one of the same session. A card gets the focus; a window comes to the front. Showing a
 * chat brings its waiting windows to the front (`showChatPrompts`). The words of the arrival announcements.
 * Depends on: the core model (asks, subagent cards, child sessions), askWordsFor, cardFocus, the announcer, goHome,
 * vocabulary, the client's `showPrompt`.
 * Called by: the shell actions, useAskAnnouncements, the composer hint, AskCard, the session view.
 * Invariants: a subagent's prompt belongs to the session that shows it (its parent); nothing here answers a prompt.
 */
import type { Ask } from "../../protocol/core";
import { announce } from "../../state/announcer";
import { editorLayoutOf } from "../../state/editorLayout";
import { askWordsFor } from "../../state/askBell";
import type { CoreState } from "../../state/coreModel";
import { formatKeys } from "../../state/shortcuts";
import { TEXT, wording } from "../../vocabulary";
import type { App } from "../shell/AppContext";
import { goHome } from "../shell/home";
import { requestCardFocus } from "./cardFocus";

/** The shortcut of "Go to the waiting prompt". */
export const GO_TO_PROMPT_KEYS = "Mod+Shift+A";
/** Longest summary an announcement reads (characters). */
const SUMMARY_MAX = 120;

/**
 * The AskCard id of `ask` (`AskCard`'s `id`): a callback's id, `protected:{id}`, a Chrome Control card's ask id;
 * `null` for asks with no card.
 */
export function cardIdOf(ask: Ask): string | null {
  if (ask.surface !== "card") return null;
  if (ask.id.startsWith("callback:")) return ask.id.slice("callback:".length);
  if (ask.id.startsWith("proposal:")) return `protected:${ask.id.slice("proposal:".length)}`;
  if (ask.kind.startsWith("chrome/ask/")) return ask.id;
  return null;
}

/** The session whose transcript shows `ask`'s card: a subagent's card is in its parent's. */
export function hostOf(core: CoreState, ask: Ask): string | null {
  const callbackId = ask.id.startsWith("callback:") ? ask.id.slice("callback:".length) : null;
  const parent = callbackId === null ? undefined : core.subagentCards[callbackId]?.parentSessionId;
  return parent ?? ask.session;
}

/** The open card asks, oldest first. */
export function waitingCards(core: CoreState): readonly Ask[] {
  return Object.values(core.asks)
    .filter((ask) => cardIdOf(ask) !== null)
    .sort((a, b) => a.openedAt - b.openedAt);
}

/**
 * True for an agent's request in a confirmation window of its own: it does not hold the app, so it can go behind
 * the main window (the person's own confirmations are modal and cannot).
 */
export function isWindowPrompt(ask: Ask): boolean {
  return ask.surface === "native" && ask.requester === "agent";
}

/** The session whose child `sessionId` is (a subagent's), if the window knows it. */
function parentOf(core: CoreState, sessionId: string): string | null {
  for (const [id, entry] of Object.entries(core.sessions)) {
    const rows = entry.state?.["childSessions"];
    if (
      Array.isArray(rows) &&
      rows.some(
        (row) =>
          row !== null &&
          typeof row === "object" &&
          !Array.isArray(row) &&
          (row["id"] ?? row["sessionId"]) === sessionId,
      )
    )
      return id;
  }
  return null;
}

/** The session that shows `ask`: a card's host (`hostOf`); a window's chat, or its parent for a subagent's. */
export function promptHost(core: CoreState, ask: Ask): string | null {
  if (!isWindowPrompt(ask) || ask.session === null) return hostOf(core, ask);
  return parentOf(core, ask.session) ?? ask.session;
}

/** Every waiting prompt (cards and windows of their own), oldest first. */
export function waitingPrompts(core: CoreState): readonly Ask[] {
  return Object.values(core.asks)
    .filter((ask) => cardIdOf(ask) !== null || isWindowPrompt(ask))
    .sort((a, b) => a.openedAt - b.openedAt);
}

/** The prompts `sessionId` shows, oldest first. */
export function promptsOf(core: CoreState, sessionId: string): readonly Ask[] {
  return waitingPrompts(core).filter((ask) => promptHost(core, ask) === sessionId);
}

/**
 * Brings the waiting windows of `sessionId` to the front, the oldest last so it ends on top (the chat was just
 * shown: select, the bell's Open session, a notification).
 */
export function showChatPrompts(app: App, sessionId: string): void {
  const core = app.core;
  if (core === null || core.client.showPrompt === undefined) return;
  const windows = promptsOf(core.store.get(), sessionId).filter(isWindowPrompt);
  for (const ask of [...windows].reverse()) void core.client.showPrompt(ask.id).catch(() => false);
}

/** Starts "showing a chat brings its waiting windows to the front" for `app`; returns the stop function. */
export function startChatPrompts(app: App): () => void {
  let selected = app.shell.store.get().selected;
  return app.shell.store.subscribe(() => {
    const now = app.shell.store.get().selected;
    if (now === selected) return;
    selected = now;
    if (now !== null) showChatPrompts(app, now);
  });
}

/** The prompt "Go to the waiting prompt" went to last (its next press goes to the one after it). */
let lastTarget: string | null = null;

function clip(text: string): string {
  const one = text.replace(/\s+/g, " ").trim();
  return one.length <= SUMMARY_MAX ? one : `${one.slice(0, SUMMARY_MAX - 1)}…`;
}

/** "Vibe asks you in “{session}”: {question} {summary}. Press Ctrl Shift A to go there." */
export function arrivedText(core: CoreState, ask: Ask): string {
  const host = hostOf(core, ask) ?? undefined;
  const words = askWordsFor(core, ask, host);
  // The command itself when the summary does not already carry it (the screen reader hears what would run).
  const what =
    words.command !== undefined && !words.summary.includes(words.command)
      ? `${words.summary}: ${words.command}`
      : words.summary;
  return wording(TEXT.askArrived, {
    session: words.session,
    question: words.question,
    summary: clip(what),
    shortcut: formatKeys(GO_TO_PROMPT_KEYS),
  });
}

/**
 * Goes to a waiting prompt: the oldest of the open session, else the oldest anywhere (its session opens; a page such
 * as Settings closes); pressed again while several wait in that session, the next one there. While the open session
 * has a prompt the steps never leave it (another chat's prompt is reached from that chat or the bell). A card gets
 * the focus, a window comes to the front. Says "No prompt is waiting." when there is none. With `sessionId` (a
 * chat's own run bar) only that chat's prompts take part.
 */
export function goToWaitingPrompt(app: App, sessionId?: string): void {
  const core = app.core?.store.get();
  const every = core === undefined ? [] : waitingPrompts(core);
  const scope = sessionId ?? app.shell.store.get().selected;
  const own = core === undefined ? [] : every.filter((ask) => promptHost(core, ask) === scope);
  const all = sessionId !== undefined || own.length > 0 ? own : every;
  if (core === undefined || all.length === 0) {
    lastTarget = null;
    announce(TEXT.askNoneWaiting);
    return;
  }
  const selected = app.shell.store.get().selected;
  const order = [
    ...all.filter((ask) => promptHost(core, ask) === selected),
    ...all.filter((ask) => promptHost(core, ask) !== selected),
  ];
  // The next one only while the person is still where the last press took them.
  const last = order.findIndex(
    (ask) => ask.id === lastTarget && promptHost(core, ask) === selected,
  );
  const target = order[(last + 1) % order.length];
  if (target === undefined) return;
  lastTarget = target.id;
  const host = promptHost(core, target);
  goHome(app);
  if (editorLayoutOf(app.shell.store.get()).slot !== "chat") app.shell.actions.unfoldChat();
  if (host !== null && host !== selected) app.shell.actions.selectSession(host);
  const id = cardIdOf(target);
  if (id !== null) requestCardFocus(id);
  else void app.core?.client.showPrompt?.(target.id).catch(() => false);
}
