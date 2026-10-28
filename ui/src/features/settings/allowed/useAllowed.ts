/**
 * React glue for the Allowed without asking page and the Safety summary: subscribes to the store and to the core
 * model's session answers, builds the entries and the page view, carries out focus requests and settles the count
 * line.
 * Depends on: react, AppContext, shell hooks, state/createStore, allowed/store, entries, view. Called by: AllowedPage,
 * AllowedSummary.
 */
import { useEffect, useMemo, useState } from "react";

import { useStore } from "../../../state/createStore";
import { useApp } from "../../shell/AppContext";
import { useCoreState } from "../../shell/hooks";
import { allowedEntries } from "./entries";
import { allowedFor, keptGroups, sessionInfos, sourcesOf } from "./store";
import { pageView, summaryView } from "./view";

/** The store, its state and the derived entries, page view and summary (the lists are read while mounted). */
export function useAllowed() {
  const app = useApp();
  const allowed = allowedFor(app);
  useEffect(() => allowed.watch(), [allowed]);
  const state = useStore(allowed.state, (s) => s);
  const grants = useCoreState((s) => s.askGrants.grants);
  const sessions = useCoreState((s) => s.sessions);
  const infos = useMemo(() => sessionInfos(sessions), [sessions]);
  const sources = useMemo(
    () => sourcesOf(grants, infos, state),
    // The sources are the grants, the sessions and the two read lists, not the rest of the page state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [grants, infos, state.rules, state.vault],
  );
  // "last 2 hours ago" is worded against the time the page opened (lazy state, so render stays pure).
  const [now] = useState(() => Date.now());
  const entries = useMemo(() => allowedEntries(sources, now), [sources, now]);
  const view = useMemo(
    () => pageView(sources, entries, state.ui, keptGroups(state)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sources, entries, state.ui, state.status, state.problem],
  );
  const summary = useMemo(() => summaryView(sources, entries), [sources, entries]);
  return { allowed, state, view, summary };
}

/** Moves focus to the first requested element on the page (`data-allowed-focus`), then clears the request. */
export function useAllowedFocus(): void {
  const allowed = allowedFor(useApp());
  const request = useStore(allowed.state, (s) => s.focus);
  useEffect(() => {
    if (request === null) return;
    const frame = requestAnimationFrame(() => {
      const all = [...document.querySelectorAll<HTMLElement>("[data-allowed-focus]")];
      for (const target of request.targets) {
        const el = all.find((e) => e.getAttribute("data-allowed-focus") === target);
        if (el !== undefined) {
          el.focus();
          break;
        }
      }
      allowed.focused();
    });
    return () => {
      cancelAnimationFrame(frame);
    };
  }, [request, allowed]);
}

/**
 * `text` while `waiting` is false; while it is true, the value it had once it stayed the same for `ms` (so a polite
 * status line is announced once after typing stops, not on every key).
 */
export function useSettled(text: string, waiting: boolean, ms = 300): string {
  const [settled, setSettled] = useState(text);
  useEffect(() => {
    if (text === settled) return;
    const timer = setTimeout(
      () => {
        setSettled(text);
      },
      waiting ? ms : 0,
    );
    return () => {
      clearTimeout(timer);
    };
  }, [text, settled, waiting, ms]);
  return waiting ? settled : text;
}
