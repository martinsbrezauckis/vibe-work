/** Model-change wording from the runtime's display names; used by checkpoint rows.
 * Vibe stores only a provider ID in old checkpoints. Unknown historical names remain generic.
 */
import type { JsonValue } from "../../../protocol/core";
import { type Obj, obj } from "./entries";

export function modelChangeText(entry: Obj, runtime: JsonValue | undefined): string {
  const id = obj(entry["details"])?.["model"];
  const models = obj(obj(runtime)?.["config"])?.["models"];
  if (!Array.isArray(models) || typeof id !== "string") return "Model changed";
  const names = new Set(
    models.flatMap((value) => {
      const model = obj(value);
      const name = model?.["displayName"];
      return (model?.["name"] === id || model?.["alias"] === id) &&
        typeof name === "string" &&
        name !== ""
        ? [name]
        : [];
    }),
  );
  const [name] = names;
  return names.size === 1 && name !== undefined ? `Model changed to ${name}` : "Model changed";
}
