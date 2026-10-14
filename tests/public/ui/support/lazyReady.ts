/**
 * Deterministic "the lazy parts are loaded" signals for the UI tests.
 *
 * The app loads several pieces on first use (`React.lazy` composer and terminal view, the composer
 * action set `composerImpl`, the `/` menu table, the markdown `remend` chunk). Under CPU load, the first
 * dynamic import in a test file can take longer than Testing Library's default 1 s `findBy*` budget, and
 * a test that reads the composer synchronously after render sees nothing yet. Two measures, both in the
 * tests (no product change):
 *  - `preloadLazy()` awaits the same dynamic imports the app uses, so by the time the shell renders the
 *    modules are cached and `lazy()` resolves in a microtask. No timer, no guess.
 *  - `composerBox()` then waits for the actual mounted element (`findByRole`); its failure bound is the
 *    generous `asyncUtilTimeout` set in setup.ts, so a slow machine waits longer instead of failing.
 * Used by: renderShell.tsx (`Rig.composer()`), transcriptRig.tsx, the composer/voice tests.
 */
import { screen } from "@testing-library/react";

/** Loads every lazily imported module the shell may need; safe to call repeatedly (module cache). */
export async function preloadLazy(
  options: { terminal?: boolean } = {},
): Promise<void> {
  await Promise.all([
    import("../../../../ui/src/features/composer/Composer"),
    import("../../../../ui/src/state/composerImpl"),
    import("../../../../ui/src/features/composer/model/slash"),
    import("../../../../ui/src/features/composer/composerRouteActions"),
    import("remend"),
    ...(options.terminal === true
      ? [import("../../../../ui/src/features/terminal/TerminalView")]
      : []),
  ]);
}

/** The mounted composer textarea, waiting for the lazy chunk (resolves when the element exists). */
export async function composerBox(): Promise<HTMLTextAreaElement> {
  return await screen.findByRole<HTMLTextAreaElement>("textbox", {
    name: "Message",
  });
}
