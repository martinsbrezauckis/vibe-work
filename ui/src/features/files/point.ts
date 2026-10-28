/**
 * File-tab context insertion. Depends on files drafts, selection readers and the composer facade;
 * called by the header, palette and shell shortcut. Points at the visible buffer, preserving draft and selection.
 */
import type { FileContent } from "../../protocol/core";
import { requestComposerFocus } from "../../state/composerFocus";
import { NEW_CHAT } from "../../state/newChatKey";
import { tabOf, viewerKind } from "../../state/files";
import type { ContextItem } from "../composer/model/message";
import type { EditorHandle } from "../editor/CodeEditor";
import type { App } from "../shell/AppContext";
import { absoluteFilePath, fileChip, sameFileChip } from "./model/context";
import { readFileSelection } from "./selection";

// Retain the full selection only for identity; long chips still quote at most 8,000 characters.
const snapshots = new WeakMap<ContextItem, string>();

/** Shared by the shortcut, File tab tooltip and palette row. */
export const POINT_FILE_KEYS = "Mod+L";

/** The app shortcut also focuses the shown message when no text File tab is mounted. */
export function registerPointShortcut(app: App): () => void {
  return app.shortcuts.register({
    id: "files.point",
    keys: POINT_FILE_KEYS,
    description: "Point Vibe at this (File tab)",
    allowInInput: true,
    run: () => {
      const point = app.commands.list().find((command) => command.id === "files.point");
      if (point !== undefined) point.run();
      else requestComposerFocus(app.shell.store.get().selected ?? NEW_CHAT);
    },
  });
}

/** Text offered by the File tab, excluding pictures, binary files and isolated viewers. */
export function canPointFile(
  content: FileContent | null,
  source: boolean,
  editing: boolean,
): boolean {
  return (
    content?.text != null &&
    content.kind !== "image" &&
    content.kind !== "binary" &&
    content.kind !== "pdf" &&
    (editing || source || !viewerKind(content.kind))
  );
}

/** Inserts one context item and returns the existing notice's words. */
export function pointFile(
  app: App,
  sessionId: string,
  root: HTMLElement | null,
  editor: EditorHandle | null,
): string | null {
  const files = app.files;
  const facade = app.composer.actions;
  if (files === null || facade === null) return null;
  const tab = tabOf(files.store.get(), sessionId);
  const draft = files.store.get().edits[sessionId];
  const content = draft?.base ?? tab.content;
  if (!canPointFile(content, tab.mode === "source", draft !== undefined) || content === null)
    return null;
  const cwd = app.core?.store.get().sessions[sessionId]?.session["cwd"];
  if (typeof cwd !== "string") return null;
  const selection =
    draft !== undefined ? editor?.selection() : root === null ? null : readFileSelection(root);
  const chip = fileChip({
    path: absoluteFilePath(content.display, cwd),
    project: cwd,
    name: content.name,
    unsaved: draft !== undefined && draft.text !== draft.base.text,
    selection: selection ?? null,
  });
  const selectedText = selection?.text ?? "";
  const duplicate = (app.composer.store.get().context[sessionId] ?? []).find((item) => {
    const snapshot = snapshots.get(item);
    if (snapshot !== undefined && selectedText !== "")
      return (
        item.ref?.path === chip.ref.path &&
        item.ref.start === chip.ref.start &&
        item.ref.end === chip.ref.end &&
        snapshot === selectedText
      );
    return sameFileChip(item, chip);
  });
  if (!duplicate) {
    const item = facade.insertContext(sessionId, chip);
    snapshots.set(item, selectedText);
  }
  requestComposerFocus(sessionId);
  return `${duplicate ? "Already in" : "Added to"} your message: ${duplicate?.label ?? chip.label}`;
}
