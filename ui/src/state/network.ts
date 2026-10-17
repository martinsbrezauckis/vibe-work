/**
 * Settings > Network: the core's `network` command (Vibe's proxy and certificate settings) and the form model
 * on top of it. The core reads and writes the settings through Vibe's `config/proxy/read|write` and answers with
 * every proxy address already masked (`http://***@host:8080` in the field, `http://host:8080` elsewhere), so nothing here ever holds a sign-in that was
 * saved; a sign-in only exists in what the user types.
 * Depends on: protocol/core types, protocol/client (errors), coreStore (`CoreController`). Called by:
 * settings/useNetwork, settings/NetworkSettings.
 * Invariants:
 * - The form starts as exactly what the core shows; a field the user did not change is not sent, so a masked address
 *   (with its hidden sign-in) is never written back by accident.
 * - An empty field the user cleared is sent as an empty string, which removes the setting.
 * - Failures are the core's own words; a declined confirmation is told apart (the user chose it, it is no error).
 */
import type { NetworkChanges, NetworkRequest, NetworkResult, NetworkView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";

/** The fields of the form, in the order the screen shows them. */
export const FIELD_NAMES = [
  "httpsProxy",
  "httpProxy",
  "allProxy",
  "noProxy",
  "certificateFile",
  "certificateFolder",
] as const;

/** One field of the form. */
export type FieldName = (typeof FIELD_NAMES)[number];

/** What the form holds: the text of each field. */
export type Form = Record<FieldName, string>;

/** Runs one `network` request. */
export async function networkCall(
  core: CoreController,
  request: NetworkRequest,
): Promise<NetworkResult> {
  const answer = await core.run({ type: "network", request });
  if (answer.type !== "network") throw new Error(`unexpected answer ${answer.type}`);
  return answer.result;
}

/** The form for a view: each field's text as the core shows it. */
export function formOf(view: NetworkView): Form {
  return {
    httpsProxy: view.httpsProxy.value,
    httpProxy: view.httpProxy.value,
    allProxy: view.allProxy.value,
    noProxy: view.noProxy.value,
    certificateFile: view.certificateFile.value,
    certificateFolder: view.certificateFolder.value,
  };
}

/** The fields whose text differs from the view's. */
export function changedFields(form: Form, view: NetworkView): FieldName[] {
  return FIELD_NAMES.filter((name) => form[name] !== view[name].value);
}

/** The request body for the changed fields only. */
export function changesOf(form: Form, view: NetworkView): NetworkChanges {
  const changes: NetworkChanges = {};
  for (const name of changedFields(form, view)) changes[name] = form[name];
  return changes;
}

/** A failed call in words, and whether the user declined the native confirmation. */
export function failure(thrown: unknown): { text: string; declined: boolean } {
  const { error } = toCallError(thrown);
  return { text: error.message, declined: error.code === "confirmationDeclined" };
}
