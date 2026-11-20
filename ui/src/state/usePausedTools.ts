/**
 * A chat's paused agent tools (today: the browser), with the person's Pause and Resume. Subscribes before reading;
 * a failed request keeps the confirmed view and shows its error inline. A read or command reply is applied only
 * when no newer event for the chat arrived meanwhile, and never over another chat's view.
 * Depends on: the core controller (`pausedTools` command and event). Called by: the chat's waiting line and the
 * built-in browser's control frame.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import type { PausedToolsView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";

/** The browser tool's name in the core's view. */
export const BROWSER_TOOL = "browser";

/** Read and follow `sessionId`'s paused tools; `pause`/`resume` act for the person. */
export function usePausedTools(core: CoreController | null, sessionId: string | null) {
  // Kept with its chat, so another chat's view is never shown after switching.
  const [held, setView] = useState<PausedToolsView | null>(null);
  const view = held !== null && held.sessionId === sessionId ? held : null;
  const [error, setError] = useState<string | null>(null);
  // Counts the events applied; a reply whose request started before the latest one is older than the view.
  const revision = useRef(0);
  useEffect(() => {
    if (core === null || sessionId === null) return;
    let live = true;
    const read = () => {
      const started = revision.current;
      void core
        .run({ type: "pausedTools", request: { op: "read", sessionId } })
        .then((result) => {
          if (live && revision.current === started && result.type === "pausedTools")
            setView(result.view);
        })
        .catch(() => {
          // A chat that is not open has nothing paused; the line stays hidden.
        });
    };
    const stop = core.listen((event) => {
      if (event.type === "pausedTools" && event.view.sessionId === sessionId) {
        revision.current++;
        setView(event.view);
      }
      if (event.type === "lagged") read();
    });
    read();
    return () => {
      live = false;
      stop();
    };
  }, [core, sessionId]);

  const act = useCallback(
    async (op: "pause" | "resume", place?: string) => {
      if (core === null || sessionId === null) return;
      setError(null);
      const started = revision.current;
      try {
        const result = await core.run({
          type: "pausedTools",
          request:
            op === "pause"
              ? { op, sessionId, ...(place === undefined ? {} : { place }) }
              : { op, sessionId },
        });
        if (result.type === "pausedTools" && revision.current === started) {
          const view = result.view;
          setView((current) =>
            current !== null && current.sessionId !== view.sessionId ? current : view,
          );
        }
      } catch (thrown) {
        setError(toCallError(thrown).error.message);
      }
    },
    [core, sessionId],
  );
  const browser = view?.paused.find((tool) => tool.tool === BROWSER_TOOL) ?? null;
  return {
    view,
    /** The browser's pause, when paused. */
    browser,
    error,
    pause: (place?: string) => act("pause", place),
    resume: () => act("resume"),
  };
}
