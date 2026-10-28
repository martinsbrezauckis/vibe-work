/**
 * The new-chat view's "Temporary chat" switch: a compact
 * switch in the folder row (it must not move the composer), and, while it is on, the words that say what happens:
 * Vibe Work deletes the chat when it quits (or at the next start after a crash); until then Vibe stores it like any
 * session, so the Vibe CLI can list it while sessions are shared with the CLI; files it changed stay changed. Vibe has
 * no "don't save" mode for one session, so this is the honest version.
 * Depends on: react. Called by: NewChatPanel.
 * Invariants: renders only; the caller owns the state (`sessionTools.newChatTemporary`).
 */
import { Switch } from "../../components/Controls";
import { term } from "../../vocabulary";
import type { ReactElement } from "react";

/** The switch (for the folder row). */
export function TemporarySwitch({
  on,
  onChange,
}: {
  on: boolean;
  onChange: (on: boolean) => void;
}): ReactElement {
  return (
    <span className="temp-toggle" data-temporary={on ? "true" : "false"}>
      <Switch
        data-testid="sessions.temporary-switch-on-change"
        aria-label="Temporary chat"
        aria-describedby={on ? "temp-chat-note" : undefined}
        aria-checked={on}
        title={`${term("workNames", "chat").tooltip} that Vibe Work deletes when it quits`}
        onClick={() => {
          onChange(!on);
        }}
      />
      <span aria-hidden="true">Temporary</span>
    </span>
  );
}

/** The sentence under the folder row while the switch is on. */
export function TemporaryNote(): ReactElement {
  return (
    <p className="copy-reason temp-note" id="temp-chat-note" role="note">
      Temporary chat: deleted when you quit Vibe Work. Until then Vibe keeps it like any chat (the
      Vibe CLI can list it while sessions are shared). Files it changes stay changed.
    </p>
  );
}
