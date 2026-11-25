/**
 * Settings › General, "Keyboard shortcuts": where the shortcuts are listed. The command palette shows every command
 * with its key combination on the right, so this row opens it instead of keeping a second list that could drift.
 * Depends on: AppContext, Controls, state/shortcuts (`formatKeys`). Called by: SettingsPage.
 * Invariants: the row's words are the catalog entry's label (`keyboard-shortcuts`); the button opens the same palette
 * as the title bar's search field and Ctrl/Cmd K.
 */
import type { ReactElement } from "react";
import { Button } from "../../components/Controls";
import { formatKeys } from "../../state/shortcuts";
import { useApp } from "../shell/AppContext";
import { ACT_SHOW_COMMANDS } from "./actionLabels";

/** The section. */
export function KeyboardShortcutsSetting(): ReactElement {
  const { shell } = useApp();
  return (
    <section aria-labelledby="keyboard-shortcuts-title" data-setting="keyboard-shortcuts">
      <h3 id="keyboard-shortcuts-title">Keyboard shortcuts</h3>
      <p className="d">
        The command palette lists every command with its shortcut. Open it here or with{" "}
        {formatKeys("Mod+K")}.
      </p>
      <Button
        data-testid="settings.keyboard-shortcuts-open"
        shape="outlined"
        onClick={() => {
          shell.actions.setOverlay("palette");
        }}
      >
        {ACT_SHOW_COMMANDS}
      </Button>
    </section>
  );
}
