/**
 * Settings, "Instructions": the personal AGENTS.md in Vibe Work's own Vibe home, for every chat.
 * Depends on: useInstructions, DocEditor, core hooks. Called by: settings/SettingsPage.
 * Invariants: Vibe Work runs its own Vibe home, so the terminal's `~/.vibe/AGENTS.md` does not apply
 * here; the page shows Vibe Work's own file (name and path) and offers a one-way add of the terminal's text below the
 * editor's (never a link, never an edit of the terminal's files). Nothing is written until Save.
 */
import { Button } from "../../components/Controls";
import { useEffect } from "react";

import { useCoreState } from "../shell/hooks";
import { DocEditor } from "./DocEditor";
import { useInstructionsController, useInstructionsState } from "./useInstructions";

/** The section (renders nothing without a core). */
export function InstructionsSettings() {
  const controller = useInstructionsController();
  const running = useCoreState((s) => s.server.phase === "running");
  const cliAvailable = useInstructionsState((s) => s.personal.doc?.cliAvailable === true);

  useEffect(() => {
    if (controller === null || !running) return;
    void controller.load("personal");
  }, [controller, running]);

  if (controller === null || !running) return null;

  return (
    <section aria-labelledby="instructions-title" data-setting="instructions">
      <h3 id="instructions-title">Instructions</h3>
      <DocEditor
        scope="personal"
        title="AGENTS.md"
        extra={
          cliAvailable ? (
            <Button
              data-testid="settings.instructions-settings-load-cli"
              type="button"
              shape="outlined"
              onClick={() => {
                void controller.loadCli();
              }}
            >
              Add my terminal's AGENTS.md
            </Button>
          ) : null
        }
      />
    </section>
  );
}
