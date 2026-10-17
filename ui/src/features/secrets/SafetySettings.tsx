/**
 * Settings, "Safety": the reminders that stop an agent's command before it runs
 * (deleting your home or a drive, disk tools, shutdown), each with a switch, and rules of your own ("a command that
 * contains..."). Reminders, not a sandbox: the agent runs as you; they catch the common honest forms of a mistake.
 * Git (force push, reset --hard, clean, branch -D in your folder) is the git guard's: it asks you first.
 * Depends on: AppContext, state/secrets (`safety`). Called by: settings/SettingsPage.
 * Invariants: every change goes through the core (which writes the settings file the hook reads), so the next
 * command is checked with it; the screen shows the core's answer, never its own guess.
 */
import { Button } from "../../components/Controls";
import { useEffect, useState } from "react";

import type { SafetyResult } from "../../protocol/core";
import { problem, safety } from "../../state/secrets";
import { useApp } from "../shell/AppContext";

/** The section. */
export function SafetySettings() {
  const { core } = useApp();
  const [view, setView] = useState<SafetyResult | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (core === null) return;
    let live = true;
    safety(core, { op: "read" })
      .then((v) => {
        if (live) setView(v);
      })
      .catch((thrown: unknown) => {
        if (live) setError(problem(thrown));
      });
    return () => {
      live = false;
    };
  }, [core]);

  if (core === null) return null;

  const change = (request: Parameters<typeof safety>[1]) => {
    setBusy(true);
    setError(null);
    safety(core, request)
      .then((v) => {
        setView(v);
      })
      .catch((thrown: unknown) => {
        setError(problem(thrown));
      })
      .finally(() => {
        setBusy(false);
      });
  };

  return (
    <section aria-labelledby="safety-title">
      <h3 id="safety-title">Safety reminders</h3>
      <p className="key-field-hint ui-help">
        Before an agent runs a command, Vibe Work stops these and tells the agent why and what to do
        instead. They are reminders against mistakes, not a sandbox: the agent works as you. Git
        (force push, reset --hard, clean, deleting branches in your own folder) is handled by the
        git guard, which asks you first.
      </p>
      {view !== null && !view.active && (
        <p className="key-line">
          The agent tools are not running in this window, so nothing is checked right now.
        </p>
      )}
      {view?.rules.map((r) => (
        <div data-item-id={r.id} className="check" key={r.id} data-setting={`safety-${r.id}`}>
          <span>{r.label}</span>
          <button
            data-testid="secrets.safety-settings-change"
            type="button"
            className="switch"
            role="switch"
            aria-label={r.label}
            aria-checked={r.on}
            disabled={busy}
            onClick={() => {
              change({ op: "setRule", id: r.id, on: !r.on });
            }}
          />
        </div>
      ))}
      <h4 id="safety-own-title">Your own rules</h4>
      {view !== null && view.custom.length === 0 && (
        <p className="key-line">
          None yet. Add text a command must not contain, like “terraform destroy”.
        </p>
      )}
      {view?.custom.map((c) => (
        <div data-item-id={c.id} className="check" key={c.id} data-custom-rule={c.id}>
          <span>A command that contains “{c.text}”</span>
          <Button
            variant="danger"
            data-testid="secrets.safety-settings-change-2"
            type="button"
            small
            disabled={busy}
            onClick={() => {
              change({ op: "removeCustom", id: c.id });
            }}
          >
            Remove
          </Button>
        </div>
      ))}
      <form
        className="secrets-add"
        data-setting="safety-custom"
        onSubmit={(e) => {
          e.preventDefault();
          const clean = text.trim();
          if (clean === "") return;
          change({ op: "addCustom", text: clean });
          setText("");
        }}
      >
        <div className="key-field">
          <label htmlFor="safety-text">Stop a command that contains</label>
          <div className="key-field-row">
            <input
              data-testid="secrets.safety-settings-set-text"
              id="safety-text"
              value={text}
              maxLength={200}
              placeholder="terraform destroy"
              autoComplete="off"
              onChange={(e) => {
                setText(e.target.value);
              }}
            />
          </div>
        </div>
        <Button
          data-testid="secrets.safety-settings-tb-btn"
          type="submit"
          small
          disabled={busy || text.trim() === ""}
        >
          Add rule
        </Button>
      </form>
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
