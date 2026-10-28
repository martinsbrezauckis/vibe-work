/**
 * The `ui/composer` UI-control route and the composer's palette entries. The Rust
 * side lists the route name (`crates/vw-host/src/host.rs` SHELL_ROUTES). Every action calls the same
 * composer function a key press or a click calls, so the route can never do something the UI cannot:
 * - `read` (default): the composer of a session as the user sees it (draft, chips, images, queue,
 *   running, access mode, model, note, rewind dialog, `heldByPid`: another Vibe process holding the session);
 * - `type {text}`: sets the draft (what typing does);
 * - `paste {text, start?, end?}`: inserts at a draft range (default: append), folding long text into a chip;
 * - `send {mode?: send|steer|queue}`: what Enter / Tab does with the current content;
 * - `insertContext {label, source, text, ref?}`: what "Point Vibe at this" does (a chip, never typed text);
 * - `attachImage {name, data}`: what pasting an image does (`data` = base64 of the file's bytes);
 * - `stop`, `retry {instructions?}`, `slash {name, args?}`, `access {mode}`, `model {alias?, effort?}`,
 *   `queueEdit {queueItemId, text}`, `queueRemove {queueItemId}`, `rewind {entryId, confirm?: here|new}`,
 *   `rewindClose`, `branch {entryId}`.
 * Depends on: AppContext; the handler (`composerRouteActions.ts`) and the `/` table load on first use.
 * Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); `sessionId` defaults to the selected session, else the
 * new-chat view's composer (`"new-chat"`): there `send` starts the session first and answers with
 * `outcome.started`; no action exists here that the composer's UI does not offer.
 */
import type { App } from "../shell/AppContext";

/** Registers the route and the palette entries; returns the function that removes them. */
export function registerComposerRoutes(app: App): () => void {
  const removeRoute = app.uiControl.register("composer", async (body) => {
    const { handleComposer } = await import("./composerRouteActions");
    return handleComposer(app, body);
  });
  // Palette entries: the GUI `/` commands for the selected session. The table loads with its chunk.
  const removers: Array<() => void> = [];
  let stopped = false;
  void import("./model/slash").then(({ SLASH_COMMANDS }) => {
    if (stopped) return;
    for (const row of SLASH_COMMANDS) {
      removers.push(
        app.commands.register({
          id: `composer.${row.name}`,
          label: `/${row.name}: ${row.description}`,
          section: "Composer",
          keywords: row.runs,
          run: () => {
            const selected = app.shell.store.get().selected;
            if (selected !== null) void app.composer.actions?.runCommand(selected, row.name, "");
          },
        }),
      );
    }
  });
  return () => {
    stopped = true;
    removeRoute();
    for (const remove of removers) remove();
  };
}
