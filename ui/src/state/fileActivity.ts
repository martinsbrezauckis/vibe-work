/** Marks open file tabs after a completed Vibe write. Uses existing history accessors; called by the file workspace. No buffer or disk content is changed. */
import type { CoreController } from "./coreStore";
import type { FileWorkspace } from "./fileWorkspace";
import { absoluteFilePath } from "../features/files/model/context";
import {
  arr,
  entryId,
  inputOf,
  kindOf,
  obj,
  outputOf,
  statusOf,
  str,
} from "../features/transcript/model/entries";

export function watchFileActivity(
  core: CoreController,
  sessions: () => Readonly<Record<string, FileWorkspace>>,
  mark: (session: string, file: string) => void,
): () => void {
  const seen = new Set<string>();
  const normalize = (path: string) => path.replaceAll("\\", "/");
  return core.listen((event) => {
    if (event.type === "snapshot") {
      const history = arr(obj(event.snapshot.state)?.["history"]);
      for (const item of history) {
        const entry = obj(item);
        if (
          entry &&
          statusOf(entry) === "completed" &&
          ["file_edit", "file_write"].includes(kindOf(entry))
        )
          seen.add(`${event.snapshot.sessionId}\u0000${entryId(entry)}`);
      }
      return;
    }
    if (
      event.type !== "change" ||
      (event.change.kind !== "entryUpserted" && event.change.kind !== "entryPatched")
    )
      return;
    const change = event.change;
    const history = arr(core.store.get().sessions[change.sessionId]?.state?.["history"]);
    const entry =
      change.kind === "entryUpserted"
        ? obj(change.entry)
        : history.map(obj).find((e) => e && entryId(e) === change.entryId);
    if (
      !entry ||
      statusOf(entry) !== "completed" ||
      !["file_edit", "file_write"].includes(kindOf(entry))
    )
      return;
    const token = `${change.sessionId}\u0000${entryId(entry)}`;
    if (seen.has(token)) return;
    seen.add(token);
    const input = inputOf(entry);
    const output = outputOf(entry);
    const path = str(output?.["filePath"]) ?? str(output?.["file"]) ?? str(input?.["filePath"]);
    const cwd = core.store.get().sessions[change.sessionId]?.session["cwd"];
    if (path === null || typeof cwd !== "string") return;
    const absolute = normalize(absoluteFilePath(path, cwd));
    for (const [session, view] of Object.entries(sessions())) {
      const folder = core.store.get().sessions[session]?.session["cwd"];
      if (typeof folder !== "string") continue;
      for (const [file, doc] of Object.entries(view.documents))
        if (
          doc.saved === undefined &&
          normalize(absoluteFilePath(doc.shown.display, folder)) === absolute
        )
          mark(session, file);
    }
  });
}
