/**
 * Secret geometry for screenshots, after the page has a viewport and locked fields have painted twice.
 * Depends on secretSafety and UI route errors; called by inspectRoutes. A failed wait releases its lock.
 */
import { UiControlError } from "../../state/uiControl";
import { captureLock, secretRects, setCaptureLock } from "./secretSafety";

const READY_MS = 2_000;
const hasViewport = () => document.readyState !== "loading" && window.innerWidth >= 1;

/** A timer bounds loading and paint together; a missing frame never counts as a paint. */
function readyForCapture(paint: boolean): Promise<void> {
  return new Promise((resolve, reject) => {
    let frame: number | undefined;
    let poll: ReturnType<typeof setTimeout> | undefined;
    const clear = () => {
      clearTimeout(timeout);
      clearTimeout(poll);
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
    const done = () => {
      clear();
      resolve();
    };
    const timeout = setTimeout(() => {
      clear();
      reject(
        new UiControlError(
          "timeout",
          hasViewport()
            ? "the page did not paint the secret fields in time"
            : "the page has not loaded a viewport",
        ),
      );
    }, READY_MS);
    const check = () => {
      if (!hasViewport()) {
        poll = setTimeout(check, 25);
      } else if (!paint) {
        done();
      } else {
        frame = requestAnimationFrame(() => {
          frame = requestAnimationFrame(() => {
            if (hasViewport()) done();
            else check();
          });
        });
      }
    };
    check();
  });
}

/** Returns geometry under the capture lock; an unlock answer is read before the lock is released. */
export async function screenshotSecrets(lock: unknown) {
  if (lock === true) setCaptureLock(true);
  try {
    await readyForCapture(lock === true);
    return {
      ...secretRects(),
      ...captureLock(),
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
    };
  } catch (error) {
    if (lock === true) setCaptureLock(false);
    throw error;
  } finally {
    if (lock === false) setCaptureLock(false);
  }
}
