/**
 * Open File-tab drafts and disk-change reconciliation. Depends on FileContent; called by files.ts.
 * A dirty draft retains the fingerprint it was based on; observing newer disk text never authorizes a write.
 */
import type { FileContent } from "../protocol/core";

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
  };
}

/** Reconcile a live disk read. Clean buffers follow disk; dirty ones preserve text and baseline. */
export function observeFileEdit(edit: FileEdit, disk: FileContent): FileEdit {
  disk = editableContent(disk);
  if (edit.base.file !== disk.file || edit.disk.fingerprint === disk.fingerprint) return edit;
  if (
    disk.text !== null &&
    !disk.truncated &&
    (edit.text === edit.base.text || edit.text === disk.text)
  )
    return {
      ...edit,
      base: disk,
      disk,
      text: disk.text,
      conflict: false,
      problem: null,
      compare: false,
    };
  return { ...edit, disk, conflict: true };
}
