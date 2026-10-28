/** App-owned GitHub reader preference and status; Settings uses typed commands and polls for login changes. */
import { useEffect, useRef, useState } from "react";
import type { GithubReadsView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";

/** Keep the confirmed switch value while starting, and show failures inline. */
export function useGithubReads(core: CoreController | null) {
  const busy = useRef(false);
  const revision = useRef(0);
  const [view, setView] = useState<GithubReadsView | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (core === null) return;
    let live = true;
    const read = () => {
      if (busy.current) return;
      const started = revision.current;
      void core
        .run({ type: "githubReads", request: { op: "read" } })
        .then((result) => {
          if (
            live &&
            !busy.current &&
            started === revision.current &&
            result.type === "githubReads"
          ) {
            setView(result.view);
            setError(null);
          }
        })
        .catch((thrown: unknown) => {
          if (live) setError(toCallError(thrown).error.message);
        });
    };
    read();
    const timer = setInterval(read, 5000);
    const stop = core.listen((event) => {
      if (event.type === "githubReads") {
        revision.current++;
        setView(event.view);
      }
    });
    return () => {
      live = false;
      clearInterval(timer);
      stop();
    };
  }, [core]);
  const change = async (enabled: boolean) => {
    if (core === null || view === null || pending) return;
    busy.current = true;
    revision.current++;
    setPending(true);
    setError(null);
    try {
      const result = await core.run({ type: "githubReads", request: { op: "write", enabled } });
      if (result.type === "githubReads") setView(result.view);
    } catch (thrown) {
      setError(toCallError(thrown).error.message);
      setView((current) => (current === null ? null : { ...current, enabled: false }));
    } finally {
      busy.current = false;
      setPending(false);
    }
  };
  return { view, pending, error, change };
}
