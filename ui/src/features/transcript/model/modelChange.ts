/** Model-change wording from the runtime's display names; used by checkpoint rows.
 * Vibe stores only a provider ID in old checkpoints. Unknown historical names remain generic.
 * Also decides which model-change checkpoints the transcript shows (`collapseModelChanges`).
 */
import type { JsonValue } from "../../../protocol/core";
import { type Obj, entryType, obj } from "./entries";

/**
 * Divider text of a model-change entry: "Model changed to <name>" when exactly one display name in the runtime's
 * model list matches the entry's model, otherwise "Model changed".
 */
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

const isModelChange = (entry: Obj): boolean =>
  entryType(entry) === "checkpoint" && entry["kind"] === "model_change";

/** The wire name a model-change checkpoint names, or `null` when it names none that can be read. */
function changedTo(entry: Obj): string | null {
  const model = obj(entry["details"])?.["model"];
  return typeof model === "string" && model !== "" ? model : null;
}

/** An entry that is part of a conversation. Checkpoints and notices take no turn. */
const inConversation = (entry: Obj): boolean => {
  const type = entryType(entry);
  return type !== "checkpoint" && type !== "notice";
};

/**
 * The entries without the model-change checkpoints that do not mark a real change of model.
 * Vibe writes a `model_change` checkpoint whenever its session is handed a model other than the one it remembers:
 * the person's pick, but also the stale pin it re-derives when a chat is resumed and the replay of a pick no turn
 * has used yet. Those entries are true to Vibe's journal and stay in the raw history; the transcript shows
 * "Model changed" only when a turn runs on a different model than the turn before it:
 * - of a run of model changes with no conversation entry between them only the last one counts (its net result);
 * - that net result is dropped when it names the model the last shown line already named;
 * - a change whose model cannot be read is always shown, and nothing is compared across it.
 * The first line of a chat is always shown: the model its first turn ran on is not recorded in the history.
 */
export function collapseModelChanges(entries: readonly Obj[]): Obj[] {
  const hidden = new Set<Obj>();
  let shown: string | null = null;
  let run: Obj[] = [];
  const settle = (): void => {
    let segment: Obj[] = [];
    const close = (): void => {
      const last = segment[segment.length - 1];
      if (last === undefined) return;
      const model = changedTo(last);
      if (model === shown) segment.forEach((entry) => hidden.add(entry));
      else {
        segment.slice(0, -1).forEach((entry) => hidden.add(entry));
        shown = model;
      }
      segment = [];
    };
    for (const entry of run) {
      if (changedTo(entry) === null) {
        close();
        shown = null;
      } else segment.push(entry);
    }
    close();
    run = [];
  };
  for (const entry of entries) {
    if (isModelChange(entry)) run.push(entry);
    else if (inConversation(entry)) settle();
  }
  settle();
  return hidden.size === 0 ? [...entries] : entries.filter((entry) => !hidden.has(entry));
}
