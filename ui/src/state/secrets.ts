/**
 * Project secrets and safety reminders for Settings: thin calls to the core's `secrets` and `safety`
 * commands, plus the plain words the screens show for the broker's decisions.
 * Depends on: protocol/core types, coreStore. Called by: features/secrets (ProjectSecretsSettings, SafetySettings).
 * Invariants: no function here receives a secret value back; `addSecret` sends the value once (the core answers
 * metadata only) and the caller clears its field right after.
 */
import type {
  CommandResult,
  SafetyRequest,
  SafetyResult,
  SecretsRequest,
  SecretsResult,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";

/** Runs one `secrets` request. */
export async function secrets(
  core: CoreController,
  request: SecretsRequest,
): Promise<SecretsResult> {
  const result: CommandResult = await core.run({ type: "secrets", request });
  if (result.type !== "secrets") throw new Error(`unexpected answer ${result.type}`);
  return result.result;
}

/** Runs one `safety` request. */
export async function safety(core: CoreController, request: SafetyRequest): Promise<SafetyResult> {
  const result: CommandResult = await core.run({ type: "safety", request });
  if (result.type !== "safety") throw new Error(`unexpected answer ${result.type}`);
  return result.result;
}

/** The message of a failed call, in the core's words. */
export function problem(thrown: unknown): string {
  return toCallError(thrown).error.message;
}

/** A time as the user's local date and time. */
export function when(ms: number): string {
  return new Date(ms).toLocaleString();
}
