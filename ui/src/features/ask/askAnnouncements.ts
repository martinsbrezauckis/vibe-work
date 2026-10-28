/** Announces newly waiting cards when focus stays elsewhere. Uses the core store and announcer; called by shell wiring. */
import type { Ask } from "../../protocol/core";
import { announce } from "../../state/announcer";
import type { App } from "../shell/AppContext";
import { arrivedText, cardIdOf, waitingCards } from "./waiting";

/** How long after arrival the focus is checked: the card has been drawn and took the focus if it could (ms). */
const ARRIVAL_CHECK_MS = 400;

/** True when the focus is inside card `id` (`AskCard`'s `data-card-id`). */
function focusIn(id: string): boolean {
  const active = document.activeElement;
  const card = active instanceof HTMLElement ? active.closest("[data-card-id]") : null;
  return card?.getAttribute("data-card-id") === id;
}

/** Starts the announcements for `app`; returns the stop function. */
export function startAskAnnouncements(app: App): () => void {
  const core = app.core;
  if (core === null) return () => undefined;
  const seen = new Set(Object.keys(core.store.get().asks));
  const timers = new Set<ReturnType<typeof setTimeout>>();
  const later = (ms: number, run: () => void) => {
    const timer = setTimeout(() => {
      timers.delete(timer);
      run();
    }, ms);
    timers.add(timer);
    return timer;
  };
  const stillOpen = (ask: Ask): Ask | undefined => core.store.get().asks[ask.id];
  let asks = core.store.get().asks;
  const sync = () => {
    const state = core.store.get();
    if (state.asks === asks) return;
    asks = state.asks;
    for (const ask of waitingCards(state)) {
      const firstSight = !seen.has(ask.id);
      if (firstSight) {
        seen.add(ask.id);
        const id = cardIdOf(ask);
        later(ARRIVAL_CHECK_MS, () => {
          const now = stillOpen(ask);
          if (now === undefined || id === null || focusIn(id) || !document.hasFocus()) return;
          announce(arrivedText(core.store.get(), now));
        });
      }
    }
  };
  const unsubscribe = core.store.subscribe(sync);
  return () => {
    unsubscribe();
    for (const timer of timers) clearTimeout(timer);
  };
}
