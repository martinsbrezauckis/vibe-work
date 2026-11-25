/**
 * The fields of a command-started MCP server: the program, the arguments (one per row), an optional working folder
 * (typed or chosen), and the environment variables (name and plain value per row). Shared by "Add MCP server" and
 * "Edit command".
 * Depends on: AppContext (folder picker), model/commandDraft. Called by: AddServerDialog, CommandServerPart.
 * Invariants: every control has its own name for keyboard and screen readers (`Argument 1`, `Variable 1 value`);
 * the saved-as-plain-text fact is stated beside the variables; a hidden value is shown as hidden, never filled in.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { useApp } from "../shell/AppContext";
import { ACT_CHOOSE_FOLDER } from "../settings/actionLabels";
import type { CommandDraft } from "./model/commandDraft";

/** The command fields; `onChange` takes a function of the latest draft (a React state setter fits). */
export function CommandSection({
  draft,
  onChange,
}: {
  draft: CommandDraft;
  onChange: (update: (previous: CommandDraft) => CommandDraft) => void;
}): ReactElement {
  const { core } = useApp();
  /** Applies a change to the latest draft (two quick clicks never overwrite each other). */
  const set = (change: (previous: CommandDraft) => Partial<CommandDraft>) => {
    onChange((previous) => ({ ...previous, ...change(previous) }));
  };
  const chooseFolder = () => {
    void core?.client
      .pickFolder()
      .then((folder) => {
        if (folder !== null) set(() => ({ cwd: folder }));
      })
      .catch(() => undefined);
  };
  return (
    <>
      <label className="cfield">
        <span>Command</span>
        <input
          data-testid="connectors.command-set-command"
          data-dialog-initial-focus="true"
          value={draft.command}
          placeholder="npx, uvx, or a full path"
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => {
            set(() => ({ command: e.target.value }));
          }}
        />
      </label>
      <fieldset className="cchoice">
        <legend>Arguments (one per row)</legend>
        {draft.args.map((arg, i) => (
          <div className="crow" key={i} data-item-id={`argument-${String(i + 1)}`}>
            <input
              data-testid="connectors.command-set-argument"
              aria-label={`Argument ${String(i + 1)}`}
              value={arg}
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => {
                const value = e.target.value;
                set((d) => ({ args: d.args.map((a, j) => (j === i ? value : a)) }));
              }}
            />
            <Button
              data-testid="connectors.command-remove-argument"
              type="button"
              shape="outlined"
              aria-label={`Remove argument ${String(i + 1)}`}
              onClick={() => {
                set((d) => ({ args: d.args.filter((_, j) => j !== i) }));
              }}
            >
              Remove
            </Button>
          </div>
        ))}
        <Button
          data-testid="connectors.command-add-argument"
          type="button"
          shape="outlined"
          onClick={() => {
            set((d) => ({ args: [...d.args, ""] }));
          }}
        >
          Add argument
        </Button>
      </fieldset>
      <div className="cfield">
        <label htmlFor="command-working-folder">Working folder (optional)</label>
        <div className="cfield-row">
          <input
            id="command-working-folder"
            data-testid="connectors.command-set-folder"
            value={draft.cwd}
            placeholder="Vibe's own folder"
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => {
              set(() => ({ cwd: e.target.value }));
            }}
          />
          <Button
            data-testid="connectors.command-choose-folder"
            type="button"
            shape="outlined"
            onClick={chooseFolder}
          >
            {ACT_CHOOSE_FOLDER}
          </Button>
        </div>
      </div>
      <fieldset className="cchoice">
        <legend>Environment variables</legend>
        <p className="cmuted">
          Values are saved as plain text in Vibe's settings file. A key or token is refused: use an
          API key server or the server's own sign-in.
        </p>
        {draft.env.map((row, i) => (
          <div className="crow" key={i} data-item-id={`variable-${String(i + 1)}`}>
            <input
              data-testid="connectors.command-set-variable-name"
              aria-label={`Variable ${String(i + 1)} name`}
              value={row.name}
              spellCheck={false}
              autoComplete="off"
              readOnly={row.hidden}
              onChange={(e) => {
                const value = e.target.value;
                set((d) => ({ env: d.env.map((r, j) => (j === i ? { ...r, name: value } : r)) }));
              }}
            />
            <input
              data-testid="connectors.command-set-variable-value"
              aria-label={`Variable ${String(i + 1)} value`}
              value={row.hidden ? "" : row.value}
              placeholder={row.hidden ? "Hidden: kept as saved" : ""}
              disabled={row.hidden}
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => {
                const value = e.target.value;
                set((d) => ({ env: d.env.map((r, j) => (j === i ? { ...r, value } : r)) }));
              }}
            />
            <Button
              data-testid="connectors.command-remove-variable"
              type="button"
              shape="outlined"
              aria-label={`Remove variable ${String(i + 1)}`}
              onClick={() => {
                set((d) => ({ env: d.env.filter((_, j) => j !== i) }));
              }}
            >
              Remove
            </Button>
          </div>
        ))}
        <Button
          data-testid="connectors.command-add-variable"
          type="button"
          shape="outlined"
          onClick={() => {
            set((d) => ({ env: [...d.env, { name: "", value: "", hidden: false }] }));
          }}
        >
          Add variable
        </Button>
      </fieldset>
    </>
  );
}
