/**
 * The file menu every place shares (a file name in the chat, a Files row, a search hit, a file tab, a Changes
 * row): one list of items in the one order (open, Point Vibe at this, versions, copy and show) and the actions
 * behind them. Items that cannot apply are left out. Every action reports its outcome (the composer note, a
 * toast, or the opened tab).
 * Depends on: files (open, edit, compare), composer (chips), changes (Show changes), the Files tab bridge,
 * RowMenu (items). Called by: FileRefLink, the Files tab rows, FilePanelTabs and the Changes rows.
 * Invariant: a file opens where its owner is: a session, or the New chat screen's folder (`files-workspace:`).
 */
import { writeClipboard } from "../../state/clipboard";
import type { MenuItem } from "../../components/RowMenu";
import { menuGroups } from "../../components/RowMenu";
import { toCallError } from "../../protocol/client/types";
import type { FileKind } from "../../protocol/core";
import { requestComposerFocus } from "../../state/composerFocus";
import { NEW_CHAT } from "../../state/newChatKey";
import type { Shown } from "../../state/files";
import { filesOf } from "../explorer/filesBridge";
import type { App } from "../shell/AppContext";
import { editableKind } from "./fileRefActions";
import {
  absoluteFilePath,
  type CodeRef,
  type CodeSelection,
  fileChip,
  sameFileChip,
} from "./model/context";

/** Where a file opens before any session: the New chat screen with that folder. */
const WORKSPACE = "files-workspace:";

/** A file a menu acts on. */
export interface FileTarget {
  /** Where it opens: a session id, or `files-workspace:<folder>` (the New chat screen's folder). */
  owner: string;
  /** The folder relative paths count from (the session's or the Files tab's folder). */
  folder: string;
  /** The path as shown: relative to `folder` inside it, else absolute. */
  path: string;
  line?: number | null;
  /** The file's kind when known (pictures and binary files have no editor). */
  kind?: FileKind | null;
  /** The resolved handle when the place already has one (a chat link). */
  shown?: Shown | null;
}

/** The menu's header line: path, or path:line. */
export function fileHeading(target: FileTarget): string {
  return target.line == null ? target.path : `${target.path}:${String(target.line)}`;
}

const absolute = (target: FileTarget) => absoluteFilePath(target.path, target.folder);
const workspace = (target: FileTarget) => target.owner.startsWith(WORKSPACE);

function warn(app: App, error: unknown): void {
  app.toasts.push({ kind: "warn", bellKind: "failure", text: toCallError(error).error.message });
}

/** The session or New chat screen the file belongs to is shown. */
function showOwner(app: App, target: FileTarget): void {
  if (workspace(target)) {
    app.shell.actions.setNewFolder(target.folder, "session");
    app.shell.actions.selectSession(null);
  } else if (app.shell.store.get().selected !== target.owner)
    app.shell.actions.selectSession(target.owner);
}

function showFileTab(app: App): void {
  app.panelTabs.setHidden("file", false);
  app.shell.actions.openTab("file");
}

/** The file's handle, resolved like a chat link is. */
async function resolve(app: App, target: FileTarget): Promise<Shown | null> {
  if (target.shown) return target.shown;
  if (app.core === null) return null;
  const result = await app.core.run({
    type: "files",
    request: { op: "resolve", sessionId: target.owner, paths: [absolute(target)] },
  });
  const ref =
    result.type === "files" && result.result.kind === "resolved"
      ? result.result.refs[0]
      : undefined;
  if (ref?.ok !== true || ref.file === null || ref.display === null) return null;
  return { file: ref.file, display: ref.display, line: target.line ?? null };
}

async function needShown(app: App, target: FileTarget): Promise<Shown> {
  const shown = await resolve(app, target);
  if (shown === null) throw new Error("This file could not be opened. It may have moved.");
  return shown;
}

/** Open (Enter): the file in its tab, at the line when there is one. */
export async function openFile(app: App, target: FileTarget): Promise<void> {
  if (app.files === null) return;
  try {
    showOwner(app, target);
    await app.files.actions.show(target.owner, await needShown(app, target));
    showFileTab(app);
  } catch (error) {
    warn(app, error);
  }
}

/** Open to the side: the second editor beside the one that has focus. */
export async function openFileToSide(app: App, target: FileTarget): Promise<void> {
  if (app.files === null) return;
  try {
    showOwner(app, target);
    await app.files.workspace.openToSide(target.owner, await needShown(app, target));
    showFileTab(app);
  } catch (error) {
    warn(app, error);
  }
}

/** Edit: the file in its editor. */
async function editFile(app: App, target: FileTarget): Promise<void> {
  if (app.files === null) return;
  try {
    showOwner(app, target);
    await app.files.actions.showForEdit(target.owner, await needShown(app, target));
    showFileTab(app);
  } catch (error) {
    warn(app, error);
  }
}

/** The composer a point from this owner lands in. */
function composerKey(app: App, owner: string): string {
  return owner.startsWith(WORKSPACE) || owner === app.shell.store.get().newChatOwner
    ? NEW_CHAT
    : owner;
}

/**
 * Point Vibe at this: a reference chip for the whole file, or the selected lines with their text, in the composer
 * of the file's owner. Nothing is sent. Answers the note the composer shows.
 */
export function pointAtFile(
  app: App,
  target: FileTarget,
  selection?: CodeSelection | null,
): string | null {
  const chip = fileChip({
    path: absolute(target),
    project: target.folder,
    name: target.path.split("/").at(-1) ?? target.path,
    unsaved: false,
    selection: selection ?? null,
  });
  return pointChip(app, target.owner, target.folder, chip);
}

/**
 * Puts `chip` into the composer of `owner` (the New chat composer for a folder before any session, shown with
 * `folder`), unless the same chip is there; the composer says which, and gets the focus. Nothing is sent.
 */
export function pointChip(
  app: App,
  owner: string,
  folder: string,
  chip: { label: string; source: string; text: string; ref?: CodeRef },
): string | null {
  const facade = app.composer.actions;
  if (facade === null) return null;
  const key = composerKey(app, owner);
  const ref = chip.ref;
  const duplicate = (app.composer.store.get().context[key] ?? []).find((item) =>
    ref === undefined
      ? item.text === chip.text
      : sameFileChip(item, { ...chip, source: "editor", ref }),
  );
  if (duplicate === undefined) facade.insertContext(key, chip);
  const note = `${duplicate ? "Already in" : "Added to"} your message: ${duplicate?.label ?? chip.label}`;
  app.composer.note(key, note);
  if (key === NEW_CHAT && app.shell.store.get().selected !== null) {
    app.shell.actions.setNewFolder(folder, "session");
    app.shell.actions.selectSession(null);
  }
  requestComposerFocus(key);
  return note;
}

/** The session Changes entry of the file, when this session changed it. */
function changedInSession(app: App, target: FileTarget): string | null {
  if (workspace(target) || app.changes === null) return null;
  const owner = app.changes.owner(target.owner);
  const view = app.changes.store.get().views[owner];
  if (view?.repoRoot == null) return null;
  const file = absolute(target).replaceAll("\\", "/");
  const root = view.repoRoot.replaceAll("\\", "/").replace(/\/+$/, "");
  return view.files.find((f) => `${root}/${f.path}` === file)?.path ?? null;
}

/** Changed since the last commit, as the Files tab's status or the session's Changes know it. */
function changedSinceCommit(app: App, target: FileTarget): boolean {
  if (changedInSession(app, target) !== null) return true;
  const files = filesOf(app)?.explorer.store.get();
  if (files?.folder !== target.folder) return false;
  return (
    files.status?.files.some((f) => f.path === target.path && f.status !== "untracked") ?? false
  );
}

/** The folder has version history the Files tab can list. */
function hasHistory(app: App, target: FileTarget): boolean {
  const files = filesOf(app)?.explorer.store.get();
  return files === undefined || files.folder !== target.folder || files.status !== null;
}

/** Compare with this commit (Files > History): the file as it was in `commit` read-only beside the file now. */
export async function compareWithCommit(
  app: App,
  target: FileTarget,
  commit: string,
): Promise<void> {
  const explorer = filesOf(app)?.explorer;
  if (app.files === null || explorer === undefined) return;
  try {
    const read = await explorer.readAt(commit, target.path);
    if (read.text === null) {
      app.toasts.push({
        kind: "warn",
        bellKind: "failure",
        text: read.note ?? "This commit could not be read.",
      });
      return;
    }
    showOwner(app, target);
    await app.files.actions.show(target.owner, await needShown(app, target));
    showFileTab(app);
    app.files.workspace.compare(target.owner, read.text, "commit", commit.slice(0, 7));
  } catch (error) {
    warn(app, error);
  }
}

/** Show changes: the Changes tab with this file selected. */
function showChanges(app: App, target: FileTarget): void {
  const path = changedInSession(app, target);
  if (path === null || app.changes === null) return;
  showOwner(app, target);
  app.panelTabs.setHidden("changes", false);
  app.shell.actions.openTab("changes");
  app.changes.select(app.changes.owner(target.owner), path);
}

/** Compare with saved version: the last commit's text read-only on the left, the file on the right. */
async function compareWithSaved(app: App, target: FileTarget): Promise<void> {
  if (app.files === null) return;
  await openFile(app, target);
  await app.files.workspace.compareSaved(target.owner);
}

/** The Files tab for `target`'s folder, shown. */
function filesTab(app: App, target: FileTarget) {
  const files = filesOf(app);
  if (files === null) return null;
  files.showFolder(target.folder);
  app.panelTabs.setHidden("git", false);
  app.shell.actions.openTab("git");
  return files.explorer;
}

/** History of this file: Files > History with only the commits that changed it. */
function historyOfFile(app: App, target: FileTarget): void {
  filesTab(app, target)?.historyOf(target.path);
}

/** Reveal in Files: the Files tab with the file's row selected. */
export function revealInFiles(app: App, target: FileTarget): void {
  filesTab(app, target)?.reveal(target.path);
}

/** Copies `text`; the toast says what was copied. */
export async function copyText(app: App, text: string, what = text): Promise<void> {
  try {
    await writeClipboard(text);
    app.toasts.feedback(`Copied ${what}.`);
  } catch (error) {
    warn(app, error);
  }
}

/** Show in folder: the system's file manager with the file selected. */
export async function showInFolder(app: App, target: FileTarget): Promise<void> {
  if (app.files === null) return;
  try {
    const error = await app.files.actions.external(
      target.owner,
      "reveal",
      (await needShown(app, target)).file,
    );
    if (error !== null) throw new Error(error);
  } catch (thrown) {
    warn(app, thrown);
  }
}

/** Where the menu is: a few items differ by place. */
export type FilePlace = "chat" | "files" | "search" | "changes";

/** The file menu's items for `target` at `place`. */
export function fileMenuItems(app: App, target: FileTarget, place: FilePlace): MenuItem[] {
  const run = (action: () => unknown) => () => {
    void action();
  };
  const inSession = changedInSession(app, target) !== null;
  return menuGroups(
    [
      { id: "open", label: "Open", keys: "Enter", run: run(() => openFile(app, target)) },
      {
        id: "open-side",
        label: "Open to the side",
        keys: "Mod+\\",
        run: run(() => openFileToSide(app, target)),
      },
      editableKind(target.kind ?? null) && {
        id: "edit",
        label: "Edit",
        run: run(() => editFile(app, target)),
      },
    ],
    [
      {
        id: "point",
        label: "Point Vibe at this",
        keys: "Mod+L",
        run: run(() => pointAtFile(app, target)),
      },
    ],
    [
      place !== "changes" &&
        inSession && {
          id: "changes",
          label: "Show changes",
          run: run(() => {
            showChanges(app, target);
          }),
        },
      changedSinceCommit(app, target) && {
        id: "compare",
        label: "Compare with saved version",
        run: run(() => compareWithSaved(app, target)),
      },
      filesOf(app) !== null &&
        hasHistory(app, target) && {
          id: "history",
          label: "History of this file",
          run: run(() => {
            historyOfFile(app, target);
          }),
        },
    ],
    [
      {
        id: "copy-path",
        label: "Copy path",
        run: run(() => copyText(app, absolute(target), "the path")),
      },
      absolute(target) !== target.path && {
        id: "copy-relative",
        label: "Copy relative path",
        run: run(() => copyText(app, target.path, target.path)),
      },
      place !== "files" &&
        filesOf(app) !== null && {
          id: "reveal-files",
          label: "Reveal in Files",
          run: run(() => {
            revealInFiles(app, target);
          }),
        },
      { id: "show-folder", label: "Show in folder", run: run(() => showInFolder(app, target)) },
    ],
  );
}
