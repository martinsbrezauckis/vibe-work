/** Read attribution for the scripted transport; the real host bridge validates this shared specification too. */
import contracts from "../../fixtures/ui-read-contracts.json";
import type { UiControlRequest } from "../../../../ui/src/protocol/client/types";

/** Mirrors read validation only; every route/action outside the reviewed read list remains driving. */
export function drives(request: UiControlRequest): boolean {
  const body = request.body;
  const input =
    typeof body === "object" && body !== null && !Array.isArray(body)
      ? (body as Record<string, unknown>)
      : null;
  const action = typeof input?.["action"] === "string" ? input["action"] : null;
  const rule = contracts.reads.find(
    (row) => row.route === request.name && row.action === action,
  );
  if (rule === undefined) return true;
  if (body !== null && input === null)
    throw new Error("read body must be an object");
  const fields: Readonly<Record<string, string | undefined>> = rule.fields;
  for (const [name, kind] of Object.entries(fields)) {
    if (kind === undefined) continue;
    const value = input?.[name];
    if (value === undefined && !kind.endsWith("!")) continue;
    const type = kind.replace("!", "");
    const valid =
      type === "nonempty"
        ? typeof value === "string" && value.length > 0
        : type === "bool"
          ? typeof value === "boolean"
          : typeof value === "number" &&
            Number.isFinite(value) &&
            value >= 0 &&
            (type !== "index" || Number.isSafeInteger(value));
    if (!valid) throw new Error(`invalid read field ${name}`);
  }
  for (const name of Object.keys(input ?? {})) {
    if (name === "action" && action !== null) continue;
    if (!Object.hasOwn(fields, name))
      throw new Error(`unsupported read field ${name}`);
  }
  return false;
}
