/**
 * The names of the `/` commands the app handles itself. Kept apart from the menu table
 * (`slash.ts`, loaded with the menu) because sending needs only the names, and the composer's send path
 * is in the main bundle.
 * Depends on: nothing. Called by: model/message (`classify`), model/slash.
 * Invariant: every name here has exactly one row in `slash.ts` (test `composer-model`). Anything else
 * typed after `/` is sent to Vibe as text: skills run server-side from a leading `/name`, and
 * unknown commands reach the model like in the CLI.
 */

/** GUI commands (Vibe's pass-through commands that make sense in a window). */
export const CLIENT_COMMANDS = [
  "compact",
  "clear",
  "rename",
  "rewind",
  "branch",
  "retry",
  "model",
  "thinking",
  "reload",
  "status",
  "log",
  "copy",
  "help",
  // the rest of Vibe 2.26.1's commands that make sense in a window, and its aliases.
  "reload-plugins",
  "plugins",
  "new",
  "config",
  "mcp",
  "connectors",
  "resume",
  "continue",
  "todo",
] as const;

/** One GUI command name. */
export type ClientCommand = (typeof CLIENT_COMMANDS)[number];

/** True when `/name` is a GUI command (case-sensitive, like Vibe's own command table). */
export function isClientCommand(name: string): name is ClientCommand {
  return (CLIENT_COMMANDS as readonly string[]).includes(name);
}
