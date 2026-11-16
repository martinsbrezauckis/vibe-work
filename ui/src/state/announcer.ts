/**
 * The app's one announcer: what a screen reader hears once, from anywhere (a prompt that arrived, a card's last minute,
 * an answer, a toast, a dictation error). Two regions, both always in the page (`Announcer`): polite for news,
 * assertive only for errors that stop work.
 * Depends on: createStore. Called by: the ask announcements, the card close, toasts, the voice strips; rendered by
 * features/shell/Announcer.
 * Invariants: messages are read in the order they came, one at a time per region (each stays `HOLD_MS` before the
 * next replaces it); the same text again within `REPEAT_MS` is dropped, so two places reporting one event say it once;
 * a text repeated later still changes the region (a trailing no-break space alternates), so it is read again; a
 * region quiet for `CLEAR_MS` is emptied (clearing is not read). A message said while the window did not have the
 * focus, or in the short time `holdUnheard` names after it got the focus back, is kept (at most `MAX_UNHEARD`, for
 * `UNHEARD_MS`) so `sayUnheard` can say it again once the reader presents the window's content again.
 */
import { createStore } from "./createStore";

/** How long one message stays before the next one in its region replaces it (ms). */
const HOLD_MS = 700;
/** The same text again within this time is not said twice (ms). */
const REPEAT_MS = 3000;
/** A region quiet for this long is cleared, so no stale message is left in it (ms). */
const CLEAR_MS = 5000;
/** How long a message said while the window did not have the focus may still be said again (ms). */
const UNHEARD_MS = 30_000;
/** Most such messages kept. */
const MAX_UNHEARD = 5;

/** What the two regions show now. */
export interface AnnouncerState {
  polite: string;
  assertive: string;
}

/** The text each region shows now; rendered by `Announcer`. */
export const announcerStore = createStore<AnnouncerState>({ polite: "", assertive: "" });

type Region = keyof AnnouncerState;

const queues: Record<Region, string[]> = { polite: [], assertive: [] };
const busy: Record<Region, boolean> = { polite: false, assertive: false };
const lastSaid = new Map<string, number>();
/** Messages said while the window did not have the focus, oldest first. */
let unheard: { at: number; text: string; region: Region }[] = [];
/** Until this time (performance.now) a message is kept as unheard even though the window has the focus. */
let unheardUntil = 0;
const clearTimers: Record<Region, ReturnType<typeof setTimeout> | undefined> = {
  polite: undefined,
  assertive: undefined,
};

function show(region: Region): void {
  const next = queues[region].shift();
  if (next === undefined) {
    busy[region] = false;
    clearTimers[region] = setTimeout(() => {
      announcerStore.set((s) => ({ ...s, [region]: "" }));
    }, CLEAR_MS - HOLD_MS);
    return;
  }
  clearTimeout(clearTimers[region]);
  busy[region] = true;
  announcerStore.set((s) => ({
    ...s,
    [region]: s[region] === next ? `${next}\u00a0` : next,
  }));
  setTimeout(() => {
    show(region);
  }, HOLD_MS);
}

/** Says `text` once: politely, or assertively for an error that stops work. Empty text says nothing. */
export function announce(text: string, options: { assertive?: boolean } = {}): void {
  const words = text.trim();
  if (words === "") return;
  const now = performance.now();
  const before = lastSaid.get(words);
  if (before !== undefined && now - before < REPEAT_MS) return;
  lastSaid.set(words, now);
  for (const [said, at] of lastSaid) if (now - at >= REPEAT_MS) lastSaid.delete(said);
  const region: Region = options.assertive === true ? "assertive" : "polite";
  if ((typeof document !== "undefined" && !document.hasFocus()) || now < unheardUntil)
    unheard = [...unheard, { at: now, text: words, region }].slice(-MAX_UNHEARD);
  queues[region].push(words);
  if (!busy[region]) show(region);
}

/**
 * Keeps messages said in the next `ms` as unheard although the window has the focus: right after the window got the
 * focus back, the screen reader may still present only the app's native windows (Orca after its own confirmation
 * window closed), so a message written then is not heard either.
 */
export function holdUnheard(ms: number): void {
  unheardUntil = performance.now() + ms;
}

/** Whether a message said in the last `UNHEARD_MS` waits to be said again. */
export function hasUnheard(): boolean {
  const now = performance.now();
  return unheard.some((message) => now - message.at < UNHEARD_MS);
}

/**
 * Says again, once, the messages said in the last `UNHEARD_MS` while the window did not have the focus. Called when
 * the window gets the focus back on systems whose screen reader presents a region's change only from the active
 * window (Orca), so a message said while the app's own native window was active is not lost.
 */
export function sayUnheard(): void {
  const now = performance.now();
  const recent = unheard.filter((message) => now - message.at < UNHEARD_MS);
  unheard = [];
  for (const { text, region } of recent) {
    // Still waiting in its queue: it is shown after the focus came back, so it is heard then.
    if (queues[region].includes(text)) continue;
    lastSaid.set(text, now);
    queues[region].push(text);
    if (!busy[region]) show(region);
  }
}

/** Forgets queued and recent messages (tests). */
export function resetAnnouncer(): void {
  queues.polite = [];
  queues.assertive = [];
  busy.polite = false;
  busy.assertive = false;
  clearTimeout(clearTimers.polite);
  clearTimeout(clearTimers.assertive);
  lastSaid.clear();
  unheard = [];
  unheardUntil = 0;
  announcerStore.set({ polite: "", assertive: "" });
}
