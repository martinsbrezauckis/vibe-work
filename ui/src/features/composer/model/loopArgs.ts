/**
 * The arguments of `/loop`, read like Vibe's own command (`vibe/cli/textual_ui/scheduled_loop_runner.py`):
 * `/loop <interval> <prompt>` adds a loop, `/loop` or `/loop list` shows them, `/loop cancel <id|all>` removes
 * them. Kept apart from the loops view model because `classify` (the send path, main bundle) needs it too and
 * nothing here may pull more code in.
 * Depends on: nothing. Called by: model/message (`classify`), state/composerLoops.
 * Invariants:
 * - Only the strict forms above are ours. Anything else after `/loop` (Vibe's model-driven form: "every weekday at 9
 *   check the PRs") returns `null`, so `classify` sends the line to Vibe as text, which the unified harness
 *   answers through its `cron` tool: typing `/loop ...` keeps working as before for those schedules.
 * - The interval is validated by Vibe (`parse_interval`: number + s/m/h/d, at least 30 s); only its shape is
 *   checked here, so a wrong value (`/loop 10s ...`) reaches Vibe and comes back with Vibe's own message.
 * - Pure: no I/O, no clock.
 */

/** What a `/loop` line asks for. */
export type LoopCommand =
  | { kind: "list" }
  /** `interval` as typed (Vibe reads `5m`, `2h`, `1d`, `30s`); `prompt` may be empty (reported to the user). */
  | { kind: "create"; interval: string; prompt: string }
  /** `id` is empty when the line named none (reported to the user). */
  | { kind: "cancel"; id: string }
  | { kind: "cancelAll" };

/** The shape of an interval word: digits and one unit (Vibe: `^(\d+)([smhd])$`, case-insensitive). */
const INTERVAL = /^\d+[smhd]$/i;
/** Vibe's words for removing a loop. */
const CANCEL = /^(cancel|rm|stop|delete)$/i;

/** The usage line shown with every refusal. */
export const LOOP_USAGE = "Use /loop 5m <what to repeat>, /loop list or /loop cancel <id|all>.";

/**
 * Reads the text after `/loop` (already trimmed by `classify`). Returns `null` when the text is not one of the
 * strict forms, which means "send it to Vibe as the model-driven /loop".
 */
export function parseLoopArgs(args: string): LoopCommand | null {
  const text = args.trim();
  if (text === "" || /^(list|ls)$/i.test(text)) return { kind: "list" };
  const match = /^(\S+)(?:\s+([\s\S]*))?$/.exec(text);
  const first = match?.[1] ?? "";
  const rest = (match?.[2] ?? "").trim();
  if (CANCEL.test(first)) {
    // "stop the server every 5m" is a sentence, not an id: it goes to Vibe's model.
    if (/\s/.test(rest)) return null;
    return rest.toLowerCase() === "all" ? { kind: "cancelAll" } : { kind: "cancel", id: rest };
  }
  if (INTERVAL.test(first)) return { kind: "create", interval: first, prompt: rest };
  return null;
}
