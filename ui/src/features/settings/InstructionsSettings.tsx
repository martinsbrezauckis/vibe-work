/**
 * Settings, "Instructions": the personal AGENTS.md in Vibe Work's own Vibe home, for every chat.
 * Depends on: useInstructions, DocEditor, core hooks. Called by: settings/SettingsPage.
 * Invariants: Vibe Work runs its own Vibe home, so the terminal's `~/.vibe/AGENTS.md` does not apply
 * here; the screen says so and offers a one-way copy (never a link, never an edit of the terminal's
 * files). Nothing is written until Save after reviewing the diff.
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
      <p className="key-line">
        Instructions are plain text that Vibe reads before it works. Saved changes reach a session
        that is already open when it reopens, which happens on its next message.
      </p>
      <DocEditor
        scope="personal"
        title="Your instructions"
        blurb={
          <>
            For every chat. Vibe Work keeps its own copy, separate from the Vibe command line, so
            instructions you wrote for your terminal do not apply here until you copy them.
          </>
        }
        extra={
          cliAvailable ? (
            <Button
              data-testid="settings.instructions-settings-load-cli"
              type="button"
              small
              onClick={() => {
                void controller.loadCli();
              }}
            >
              Copy my CLI instructions
            </Button>
          ) : null
        }
      />
    </section>
  );
}
