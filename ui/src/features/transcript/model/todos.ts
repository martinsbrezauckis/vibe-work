/**
 * The pinned todo strip ("`todo` updates a pinned todo strip derived from the latest todo
 * effect, like the CLI's `/todo`"). A `todo` effect (`TodoEffectDetail`) carries the whole list in
 * `state.output.todos` once it completed (`detail.input.todos` for a `write` before that).
 * Depends on: entries.ts. Called by: the transcript view.
 * Invariants: pure; unknown statuses are kept as text, never dropped.
 */
import type { JsonValue } from "../../../protocol/core";
import { type Obj, entryType, inputOf, kindOf, obj, outputOf, statusOf, str } from "./entries";

/** One todo item (`TodoEffectItem`). */
export interface Todo {
  id: string;
  content: string;
  /** `pending`, `in_progress`, `completed`, `cancelled` (or a newer value, shown as is). */
  status: string;
}

function parse(list: JsonValue | undefined): Todo[] | null {
  if (!Array.isArray(list)) return null;
  return list.flatMap((raw) => {
    const item = obj(raw);
    const content = str(item?.["content"]);
    if (item === null || content === null) return [];
    return [{ id: str(item["id"]) ?? content, content, status: str(item["status"]) ?? "pending" }];
  });
}

/** The list of the newest todo effect that carries one, or `null` when the session has none. */
export function latestTodos(history: readonly JsonValue[]): Todo[] | null {
  for (let i = history.length - 1; i >= 0; i -= 1) {
    const entry: Obj | null = obj(history[i]);
    if (entry === null || entryType(entry) !== "effect" || kindOf(entry) !== "todo") continue;
    if (statusOf(entry) === "failed" || statusOf(entry) === "cancelled") continue;
    const list = parse(outputOf(entry)?.["todos"]) ?? parse(inputOf(entry)?.["todos"]);
    if (list !== null) return list;
  }
  return null;
}

/** Done / total, for the strip's header. */
export function todoProgress(todos: readonly Todo[]): { done: number; total: number } {
  const counted = todos.filter((t) => t.status !== "cancelled");
  return { done: counted.filter((t) => t.status === "completed").length, total: counted.length };
}
