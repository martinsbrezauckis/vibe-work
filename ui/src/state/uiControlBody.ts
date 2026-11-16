/**
 * The two body contracts of the Debug API's `ui/<name>` routes, shared by every feature's route file.
 * Depends on: state/uiControl (UiControlError). Called by: the feature route registrations.
 * Invariants: both accept exactly a JSON object (not an array) and refuse anything else with `invalidParams` and the
 * same sentence; they differ only in an absent body: `bodyObject` reads a missing (`undefined`) or `null` body as `{}`
 * (routes whose fields are all optional), `requiredBodyObject` refuses it (routes that need named fields).
 */
import { UiControlError } from "./uiControl";

const NOT_AN_OBJECT = "the body must be a JSON object";

function isObject(body: unknown): body is Record<string, unknown> {
  return typeof body === "object" && body !== null && !Array.isArray(body);
}

/** The body as an object; a missing or `null` body is `{}`. */
export function bodyObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  return requiredBodyObject(body);
}

/** The body as an object; a missing or `null` body is refused like any other non-object. */
export function requiredBodyObject(body: unknown): Record<string, unknown> {
  if (isObject(body)) return body;
  throw new UiControlError("invalidParams", NOT_AN_OBJECT);
}
