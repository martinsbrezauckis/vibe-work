/**
 * The command-server part of the "Add MCP server" dialog and of "Edit command", as plain data: the program, one
 * argument per row, the working folder and the environment variables. `commandProblem` says what is missing (the core
 * checks again and resolves the program), `commandInput` turns the draft into the request's fields.
 * Depends on: state/connectors (`CommandInput`), the generated protocol types. Called by: CommandSection,
 * AddServerDialog, CommandServerPart, tests.
 * Invariants: a variable whose value is hidden (an older entry whose value looks like a key) keeps its name and is
 * sent as `keepExisting`; its value is never in the draft. Values are plain text and are saved as such.
 */
import type { CommandConfigView, CommandEnvInput } from "../../../protocol/core";
import type { CommandInput } from "../../../state/connectors";

/** One environment variable row. */
export interface EnvRow {
  name: string;
  value: string;
  /** The saved value is hidden here (it looks like a key); the row keeps it as saved. */
  hidden: boolean;
}

/** What the user filled in. */
export interface CommandDraft {
  name: string;
  command: string;
  args: string[];
  cwd: string;
  env: EnvRow[];
}

/** What the page shows for an argument the core hid (an older entry that looks like a key). */
export const HIDDEN_ARGUMENT = "[hidden]";

/** A draft with nothing in it. */
export function emptyCommandDraft(): CommandDraft {
  return { name: "", command: "", args: [], cwd: "", env: [] };
}

/** The draft of a saved command server. */
export function draftOf(name: string, config: CommandConfigView): CommandDraft {
  return {
    name,
    command: config.command,
    args: [...config.args],
    cwd: config.cwd ?? "",
    env: config.env.map((e) => ({ name: e.name, value: e.value ?? "", hidden: e.hidden })),
  };
}

const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** Why the draft cannot be sent yet, or `null`. */
export function commandProblem(draft: CommandDraft): string | null {
  if (draft.command.trim() === "")
    return "Type the program to run, such as npx or uvx, or its full path.";
  if (draft.args.includes(HIDDEN_ARGUMENT))
    return "An argument is hidden because it looks like a key. Replace it or remove it.";
  const seen = new Set<string>();
  for (const row of draft.env) {
    const name = row.name.trim();
    if (name === "" && row.value === "") continue;
    if (!ENV_NAME.test(name))
      return `The variable name ${name === "" ? "is empty" : `“${name}” is not valid`}: letters, digits and underscores, not starting with a digit.`;
    if (seen.has(name)) return `The variable ${name} is listed twice.`;
    seen.add(name);
  }
  return null;
}

/** The request fields of a draft: empty rows are left out. */
export function commandInput(draft: CommandDraft, includeName: boolean): CommandInput {
  const env: CommandEnvInput[] = draft.env
    .filter((r) => r.name.trim() !== "" || r.value !== "")
    .map((r) =>
      r.hidden
        ? { name: r.name.trim(), keepExisting: true }
        : { name: r.name.trim(), value: r.value },
    );
  const name = draft.name.trim();
  const cwd = draft.cwd.trim();
  return {
    name: includeName && name !== "" ? name : null,
    command: draft.command.trim(),
    args: draft.args.filter((a) => a !== ""),
    cwd: cwd === "" ? null : cwd,
    env,
  };
}
