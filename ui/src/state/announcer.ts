/**
 * The app's one announcer: what a screen reader hears once, from anywhere (a prompt that arrived, a card's last minute,
 * an answer, a toast, a dictation error). Two regions, both always in the page (`Announcer`): polite for news,
 * assertive only for errors that stop work.
 * Depends on: createStore. Called by: the ask announcements, the card close, toasts, the voice strips; rendered by
 * features/shell/Announcer.
 * Invariants: messages are read in the order they came, one at a time per region (each stays `HOLD_MS` before the
 * next replaces it); the same text again within `REPEAT_MS` is dropped, so two places reporting one event say it once;
 * a text repeated later still changes the region (a trailing no-break space alternates), so it is read again.
 */
import { createStore } from "./createStore";

/** How long one message stays before the next one in its region replaces it (ms). */
const HOLD_MS = 700;
/** The same text again within this time is not said twice (ms). */
const REPEAT_MS = 3000;

/** What the two regions show now. */
export interface AnnouncerState {
  polite: string;
  assertive: string;
}

export const announcerStore = createStore<AnnouncerState>({ polite: "", assertive: "" });

type Region = keyof AnnouncerState;

const queues: Record<Region, string[]> = { polite: [], assertive: [] };
const busy: Record<Region, boolean> = { polite: false, assertive: false };
const lastSaid = new Map<string, number>();

function show(region: Region): void {
  const next = queues[region].shift();
  if (next === undefined) {
    busy[region] = false;
    return;
  }
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
  const now = Date.now();
  const before = lastSaid.get(words);
  if (before !== undefined && now - before < REPEAT_MS) return;
  lastSaid.set(words, now);
  for (const [said, at] of lastSaid) if (now - at >= REPEAT_MS) lastSaid.delete(said);
  const region: Region = options.assertive === true ? "assertive" : "polite";
  queues[region].push(words);
  if (!busy[region]) show(region);
}

/** Forgets queued and recent messages (tests). */
export function resetAnnouncer(): void {
  queues.polite = [];
  queues.assertive = [];
  busy.polite = false;
  busy.assertive = false;
  lastSaid.clear();
  announcerStore.set({ polite: "", assertive: "" });
}
