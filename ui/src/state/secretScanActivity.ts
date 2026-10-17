/**
 * Forward real window activity to the background scanner's idle clock, only while enabled.
 * Depends on the typed core controller; shell wiring installs it and removes every listener/timer.
 */
import type { CoreController } from "./coreStore";

const INTERVAL_MS = 250;
const EVENTS = ["keydown", "input", "pointerdown", "wheel", "focus"] as const;

/** Connect input to the scanner without changing focus, controls or the session model. */
export function attachSecretScanActivity(core: Pick<CoreController, "run" | "listen">): () => void {
  let live = true;
  let enabled = false;
  let revision = 0;
  let lastSent = -Infinity;
  let trailing: ReturnType<typeof setTimeout> | undefined;
  const clear = () => {
    clearTimeout(trailing);
    trailing = undefined;
  };
  const send = () => {
    if (!live || !enabled) return;
    lastSent = Date.now();
    void core.run({ type: "secretScan", request: { op: "activity" } }).catch(() => {});
  };
  const activity = () => {
    if (!enabled) return;
    if (Date.now() - lastSent >= INTERVAL_MS) send();
    clear();
    // The trailing signal makes the full idle wait begin after the final input, too.
    trailing = setTimeout(send, INTERVAL_MS);
  };
  const read = () => {
    const started = revision;
    void core
      .run({ type: "secretScan", request: { op: "read" } })
      .then((result) => {
        if (live && started === revision && result.type === "secretScan") {
          enabled = result.view.settings.enabled;
          if (!enabled) clear();
        }
      })
      .catch(() => {});
  };
  const stop = core.listen((event) => {
    if (event.type === "secretScan") {
      revision += 1;
      enabled = event.view.settings.enabled;
      if (!enabled) clear();
    }
    if (event.type === "lagged" || (event.type === "server" && event.status.state === "running"))
      read();
  });
  for (const event of EVENTS)
    window.addEventListener(event, activity, { capture: true, passive: true });
  read();
  return () => {
    live = false;
    clear();
    stop();
    for (const event of EVENTS) window.removeEventListener(event, activity, true);
  };
}
