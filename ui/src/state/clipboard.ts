/** Clipboard writes for main-window controls. Uses the typed native command during Debug API actions, when WebKit has no trusted gesture.
 * Depends on the core client and drive scope; the shell registers its writer and controls call writeClipboard. Vault copy remains its own command. */
import type { CoreController } from "./coreStore";

const writers = new Set<{ core: CoreController; driving: () => boolean }>();

/** Registers only for the mounted shell; no standing Debug API authority is kept here. */
export function registerClipboard(core: CoreController, driving: () => boolean): () => void {
  const entry = { core, driving };
  writers.add(entry);
  return () => {
    writers.delete(entry);
  };
}

/** Resolves only after the platform has accepted the write; ordinary window copies keep their browser gesture. */
export async function writeClipboard(text: string): Promise<void> {
  const writer = [...writers].reverse().find((entry) => entry.driving());
  if (writer === undefined) return navigator.clipboard.writeText(text);
  const result = await writer.core.run({ type: "copyText", text });
  if (result.type !== "done") throw new Error("The clipboard write did not complete.");
}
