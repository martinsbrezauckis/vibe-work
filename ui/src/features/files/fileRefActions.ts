/** File-link actions shared by the transcript menu and Debug API; uses the files controller and app feedback. */
import { writeClipboard } from "../../state/clipboard";
import type { FileKind } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import type { Shown } from "../../state/files";
import type { App } from "../shell/AppContext";

/** The file-link menu's actions. */
export type RefAction = "open" | "edit" | "reveal" | "copy-path";
/** Kinds whose bytes may be editable after a complete read. */
export function editableKind(kind: FileKind | null): boolean {
  return kind !== "image" && kind !== "binary";
}

/** Runs exactly the same action for a visible control and the Debug API. */
export async function runFileRefAction(
  app: App,
  sessionId: string,
  shown: Shown,
  item: RefAction,
  origin: "ui" | "debug-api" = "ui",
): Promise<void> {
  const files = app.files;
  if (files === null) return;
  const actions = origin === "ui" ? files.actions : files.debugApi;
  try {
    if (item === "copy-path") {
      await writeClipboard(shown.display);
      app.toasts.feedback("Copied.");
    } else if (item === "reveal") {
      const error = await actions.external(sessionId, "reveal", shown.file);
      if (error !== null) throw new Error(error);
    } else {
      app.shell.actions.openTab("file");
      if (item === "edit") await actions.showForEdit(sessionId, shown);
      else await actions.show(sessionId, shown);
    }
  } catch (error) {
    app.toasts.push({ kind: "warn", bellKind: "failure", text: toCallError(error).error.message });
    throw error;
  }
}
