/**
 * The folder a Debug API request starts a chat in: for `ui/sessions` `projectStart` and `ui/projects` `newSession`.
 * A chat in a project that has no main folder would open the native folder picker, which nobody can answer through
 * the Debug API, so such a request names the folder (`folder`, a path) instead, or is refused by name. A named folder
 * is registered as a folder the user picked (the core's `pickFolder`, the Debug API's "Choose folder", which also
 * checks that it is a directory) before the chat starts there.
 * Depends on: coreStore (`run`), projectGroups (`NEEDS_MAIN_FOLDER`, `needsFolderPicker`), uiControl (errors),
 * client types (`toCallError`). Called by: the `sessions` and `projects` UI routes.
 * Invariants: never opens a picker; a bad `folder` is `invalidParams`; the core's own refusal keeps its code.
 */
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { NEEDS_MAIN_FOLDER } from "./projectGroups";
import { UiControlError } from "./uiControl";

/**
 * Checks `value` (the route's `folder`) for a chat in project `id` and registers it. Returns the folder to start in
 * (`undefined`: the project's own main folder).
 *
 * @throws UiControlError `invalidParams` when `folder` is not a non-empty string, or when it is absent and the
 *   project has no main folder (a picker would be needed); the core's refusal for a folder that is not a directory.
 */
export async function startFolderFor(
  core: CoreController | null,
  groups: { needsFolderPicker(id: string): boolean },
  id: string,
  value: unknown,
): Promise<string | undefined> {
  if (value === undefined) {
    if (groups.needsFolderPicker(id)) throw new UiControlError("invalidParams", NEEDS_MAIN_FOLDER);
    return undefined;
  }
  if (typeof value !== "string" || value === "")
    throw new UiControlError("invalidParams", "folder must be a non-empty path");
  if (core === null) throw new UiControlError("serverUnavailable", "the core is not running");
  try {
    await core.run({ type: "pickFolder", path: value }, { origin: "debug-api" });
  } catch (thrown) {
    const { error } = toCallError(thrown);
    throw new UiControlError(error.code, error.message);
  }
  return value;
}
