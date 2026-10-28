/** File mention tokens and picks. Depends on core hits and the file chip builder; called by the composer hook. */
import type { FileHit } from "../../../protocol/core";
import { absoluteFilePath, fileChip } from "../../files/model/context";
import type { App } from "../../shell/AppContext";

/** The typed token, ending at the caret. */
export interface Mention {
  start: number;
  end: number;
  query: string;
}

/** An @ at the start or after space, newline or ( opens a mention; whitespace ends it. */
export function mentionAt(text: string, caret: number): Mention | null {
  const match = /(?:^|[ \n(])@([^\s]*)$/.exec(text.slice(0, caret));
  if (match === null) return null;
  const query = match[1] ?? "";
  return { start: caret - query.length - 1, end: caret, query };
}

/** A core file path's name and relative folder, with the trailing slash shown in the menu. */
export function mentionName(path: string): { name: string; folder: string } {
  const split = path.lastIndexOf("/") + 1;
  return { name: path.slice(split), folder: path.slice(0, split) };
}

/** The menu shows only file names from this project's core results, at most eight. */
export function mentionRows(hits: readonly FileHit[], project: string, query: string): FileHit[] {
  return hits
    .filter(
      (hit) =>
        hit.folder === project &&
        hit.line === null &&
        mentionName(hit.path).name.toLowerCase().includes(query.toLowerCase()),
    )
    .slice(0, 8);
}

/** Replaces just the mention and inserts one whole-file reference through the existing facade. */
export function chooseMention(app: App, sessionId: string, mention: Mention, hit: FileHit): number {
  const { name } = mentionName(hit.path);
  const chip = fileChip({
    path: absoluteFilePath(hit.path, hit.folder),
    project: hit.folder,
    name,
    unsaved: false,
  });
  const duplicate = (app.composer.store.get().context[sessionId] ?? []).some(
    (item) => item.ref?.path === chip.ref.path && item.ref.start === 0 && item.ref.end === 0,
  );
  if (!duplicate) app.composer.actions?.insertContext(sessionId, chip);
  const text = app.composer.store.get().drafts[sessionId] ?? "";
  app.composer.setDraft(sessionId, text.slice(0, mention.start) + name + text.slice(mention.end));
  return mention.start + name.length;
}
