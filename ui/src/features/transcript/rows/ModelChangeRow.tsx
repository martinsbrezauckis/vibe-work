/** Resolves a model checkpoint against this chat's runtime; called by RowView.
 * Uses the core store's live model names and keeps unavailable historical IDs out of the divider.
 */
import { useCoreState } from "../../shell/hooks";
import type { Obj } from "../model/entries";
import { modelChangeText } from "../model/modelChange";
import { DividerRow } from "./SmallRows";

export function ModelChangeRow({ entry, sessionId }: { entry: Obj; sessionId: string }) {
  const runtime = useCoreState((state) => state.sessions[sessionId]?.runtime);
  return <DividerRow entry={entry} text={modelChangeText(entry, runtime)} />;
}
