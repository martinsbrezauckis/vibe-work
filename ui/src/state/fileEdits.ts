/**
 * Open File-tab drafts and disk-change reconciliation. Depends on FileContent and
 * merge3; called by files.ts and the editor's notices.
 * Invariants: a clean buffer follows disk and marks the changed lines; unsaved edits on other lines are merged (the
 * pre-merge text is kept for Undo merge); edits on the same lines keep the user's text and baseline until they
 * choose, so observing newer disk text never authorizes a write and never drops typed text.
 */
import type { FileContent } from "../protocol/core";
import { merge3 } from "./merge3";

/** What the editor tells the user about the last change on disk. */
export type DiskNotice =
  /** The buffer had no unsaved edits and now shows the disk version. */
  | { kind: "reloaded"; lines: readonly number[] }
  /**
   * The disk change was combined with unsaved edits (`chose`: how shared lines were settled by the user);
   * `mine` is the editor text before, for Undo merge.
   */
  | { kind: "merged"; lines: readonly number[]; mine: string; chose?: "mine" | "theirs" }
  /** Both changed the same lines (of the user's text): the editor is unchanged until the user chooses. */
  | { kind: "overlap"; lines: readonly number[] };

/** One open editor buffer, separate from the newest File-tab disk preview. */
export interface FileEdit {
  /** Identifies this particular draft across disk updates and asynchronous saves. */
  id: number;
  base: FileContent;
  disk: FileContent;
  text: string;
  conflict: boolean;
  problem: string | null;
  saving: boolean;
  compare: boolean;
  wrap: boolean;
  /** Lines of `text` that changed on disk while the file was open (shown when the setting is on). */
  marks: readonly number[];
  notice: DiskNotice | null;
}

let nextEditId = 0;

/** Match CodeMirror's document line endings without changing the disk preview. */
export function normalizeEditText(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

function editableContent(content: FileContent): FileContent {
  if (content.text === null) return content;
  const text = normalizeEditText(content.text);
  return text === content.text ? content : { ...content, text };
}

/** The same complete-text rule for the File tab's Edit button and file-link actions. */
export function canEditContent(content: FileContent): boolean {
  return (
    content.text !== null &&
    !content.truncated &&
    !content.tooLarge &&
    content.kind !== "image" &&
    content.kind !== "binary"
  );
}

/** A complete, editable text file starts with a clean buffer. */
export function beginFileEdit(content: FileContent): FileEdit | null {
  if (!canEditContent(content) || content.text === null) return null;
  const editable = { ...content, text: normalizeEditText(content.text) };
  return {
    id: ++nextEditId,
    base: editable,
    disk: editable,
    text: editable.text,
    conflict: false,
    problem: null,
    saving: false,
    compare: false,
    wrap: false,
    marks: [],
    notice: null,
  };
}

/** Reconcile a live disk read: follow, merge, or keep the user's text and ask. */
export function observeFileEdit(edit: FileEdit, disk: FileContent): FileEdit {
  disk = editableContent(disk);
  if (edit.base.file !== disk.file || edit.disk.fingerprint === disk.fingerprint) return edit;
  const base = edit.base.text;
  if (disk.text === null || disk.truncated || base === null)
    return { ...edit, disk, conflict: true };
  const follow = { ...edit, base: disk, disk, conflict: false, problem: null, compare: false };
  if (edit.text === disk.text) return { ...follow, text: disk.text, marks: [], notice: null };
  if (edit.text === base) {
    const lines = merge3(base, base, disk.text).theirs;
    return { ...follow, text: disk.text, marks: lines, notice: { kind: "reloaded", lines } };
  }
  const merged = merge3(base, edit.text, disk.text);
  if (merged.conflicts.length > 0)
    return {
      ...edit,
      disk,
      conflict: true,
      marks: [],
      notice: { kind: "overlap", lines: merged.conflicts },
    };
  return {
    ...follow,
    text: merged.text,
    marks: merged.theirs,
    notice: { kind: "merged", lines: merged.theirs, mine: edit.text },
  };
}

/** Got it: the marks and the notice go; the text stays. */
export function acknowledgeDiskChange(edit: FileEdit): FileEdit {
  return { ...edit, marks: [], notice: null };
}

/** Undo merge: the editor text from before the merge (the baseline stays the new disk version). */
export function undoMerge(edit: FileEdit): FileEdit {
  if (edit.notice?.kind !== "merged") return edit;
  return { ...edit, text: edit.notice.mine, marks: [], notice: null };
}

/** Keep yours / Take Vibe's: the shared lines from that side, every other change of both sides kept. */
export function resolveOverlap(edit: FileEdit, prefer: "mine" | "theirs"): FileEdit {
  const base = edit.base.text;
  const disk = edit.disk.text;
  if (base === null || disk === null || edit.disk.truncated) return edit;
  const merged = merge3(base, edit.text, disk, prefer);
  return {
    ...edit,
    base: edit.disk,
    text: merged.text,
    conflict: false,
    compare: false,
    marks: merged.theirs,
    notice: { kind: "merged", lines: merged.theirs, mine: edit.text, chose: prefer },
  };
}
