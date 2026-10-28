/**
 * Body readers for UI-control routes: a request body as an object, and typed fields that answer
 * `invalidParams` naming the field when they are missing or of the wrong type.
 * Depends on: uiControl (UiControlError). Called by: files/editorsRoute, explorer/explorerRoute.
 * Invariant: every reader throws a `UiControlError("invalidParams", ...)`, never a plain Error, so the Debug API
 * caller always gets a 400 that says what to fix.
 */
import { UiControlError } from "./uiControl";

/** The body as an object (`undefined`/`null` read as `{}`). */
export function bodyObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

/** A required non-empty string field. */
export function textField(input: Record<string, unknown>, key: string): string {
  const value = input[key];
  if (typeof value === "string" && value !== "") return value;
  throw new UiControlError("invalidParams", `${key} must be a non-empty string`);
}

/** A required boolean field. */
export function boolField(input: Record<string, unknown>, key: string): boolean {
  const value = input[key];
  if (typeof value === "boolean") return value;
  throw new UiControlError("invalidParams", `${key} must be true or false`);
}

/** A required field that must be one of `allowed`. */
export function oneOf<T extends string>(
  input: Record<string, unknown>,
  key: string,
  allowed: readonly T[],
): T {
  const value = input[key];
  if (typeof value === "string" && (allowed as readonly string[]).includes(value))
    return value as T;
  throw new UiControlError("invalidParams", `${key} must be ${allowed.join(", ")}`);
}

/** True for a body whose action is absent or `state`: a pure read, answered even while another action runs. */
export function isStateRead(body: unknown): boolean {
  if (body === undefined || body === null) return true;
  if (typeof body !== "object" || Array.isArray(body)) return false;
  const action = (body as Record<string, unknown>)["action"];
  return action === undefined || action === "state";
}

/** Polls `done` every 50 ms until it is true or `ms` passed; answers whether it became true. */
export async function settleWithin(done: () => boolean, ms: number): Promise<boolean> {
  const end = Date.now() + ms;
  while (!done()) {
    if (Date.now() > end) return false;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return true;
}
