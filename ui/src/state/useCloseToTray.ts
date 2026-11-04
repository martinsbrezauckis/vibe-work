/** Native General switch. Reads and saves through the typed client; polls while Settings is open
 * so Linux host loss removes the row. Failed saves retain the confirmed value. */
import { useEffect, useRef, useState } from "react";
import type { DesktopClient } from "../protocol/client/types";
import { toCallError } from "../protocol/client/types";

/** The close preference and current host availability. */
export function useCloseToTray(desktop: DesktopClient | undefined) {
  const revision = useRef(0);
  const [view, setView] = useState<{ enabled: boolean; available: boolean } | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (desktop === undefined) return;
    let live = true;
    const read = () => {
      const started = revision.current;
      void desktop
        .settings()
        .then((value) => {
          if (live && revision.current === started) setView(value);
        })
        .catch(() => {
          if (live) setView(null);
        });
    };
    read();
    const timer = setInterval(read, 1000);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [desktop]);
  const change = async (enabled: boolean) => {
    if (desktop === undefined || pending) return;
    revision.current++;
    setPending(true);
    try {
      setView(await desktop.settings(enabled));
      setError(null);
    } catch (thrown) {
      setError(toCallError(thrown).error.message);
    } finally {
      revision.current++;
      setPending(false);
    }
  };
  return { view, pending, error, change };
}
