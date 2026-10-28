/**
 * File context labels and snapshots. Depends on the File tab's highlighting languages; called by
 * file pointing and tests. Whole files are references, selected text is quoted data, never a command.
 */
import { languageOf } from "../../../components/markdown/highlight";
import { languageForName } from "./kinds";

/** An absolute file location; zero lines mean the whole file. */
export interface CodeRef {
  path: string;
  start: number;
  end: number;
}

/** Exact selected text and the inclusive, one-based lines it touches. */
export interface CodeSelection {
  start: number;
  end: number;
  text: string;
}

/** A file chip before the composer assigns its id. */
export interface FileChip {
  source: "editor";
  label: string;
  text: string;
  ref: CodeRef;
}

/** The core's display path is relative inside the session folder, otherwise absolute. */
export function absoluteFilePath(display: string, project: string): string {
  return /^(?:\/|[a-z]:[\\/]|\\\\)/i.test(display)
    ? display
    : `${project.replace(/[\\/]+$/, "")}/${display}`;
}

/** Builds a selection snapshot or a whole-file reference without reading or uploading the file. */
export function fileChip(input: {
  path: string;
  project: string | null;
  name: string;
  unsaved: boolean;
  selection?: CodeSelection | null;
}): FileChip {
  const project = input.project?.replace(/[\\/]+$/, "").replaceAll("\\", "/");
  const absolute = input.path.replaceAll("\\", "/");
  const inside = project != null && absolute.startsWith(`${project}/`);
  const path = inside ? absolute.slice(project.length + 1) : input.path;
  const selection = input.selection;
  if (selection == null || selection.text === "")
    return {
      source: "editor",
      label: `File · ${path}`,
      ref: { path: input.path, start: 0, end: 0 },
      text:
        `The file ${path}${inside ? " in this project" : ""}.` +
        (input.unsaved ? " It has unsaved edits in the editor that are not on disk yet." : ""),
    };
  const range = `${String(selection.start)}–${String(selection.end)}`;
  const cut = selection.text.length > 8000;
  const language = languageOf(languageForName(input.name)) ?? "";
  return {
    source: "editor",
    label:
      `${path}:${selection.start === selection.end ? String(selection.start) : range}` +
      (input.unsaved ? " · unsaved" : "") +
      (cut ? " · cut" : ""),
    ref: { path: input.path, start: selection.start, end: selection.end },
    text:
      `Selected in ${path}, lines ${range}` +
      (input.unsaved ? " (from unsaved edits in the editor; the file on disk may differ)" : "") +
      `:\n\`\`\`${language}\n${selection.text.slice(0, 8000)}\n\`\`\`` +
      (cut ? "\n(cut: the selection is longer; read the file for the rest)" : ""),
  };
}

/**
 * A folder chip: a reference to a whole folder (no line range, no File tab), never its content. The label names the
 * folder (chips without a file reference are cut at 80 characters, which would cut a long path before its name); the
 * text gives the agent the full path.
 */
export function folderChip(input: { path: string; project: string | null }): {
  source: "folder";
  label: string;
  text: string;
} {
  const project = input.project?.replace(/[\\/]+$/, "").replaceAll("\\", "/");
  const absolute = input.path.replaceAll("\\", "/");
  const inside = project != null && absolute.startsWith(`${project}/`);
  const path = inside ? absolute.slice(project.length + 1) : input.path;
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  return {
    source: "folder",
    label: `Folder · ${name}`,
    text: `The folder ${path}${inside ? " in this project" : ""}.`,
  };
}

/** Compares file identity and the quoted snapshot; labels are presentation only. */
export function sameFileChip(a: { text: string; ref?: CodeRef }, b: FileChip): boolean {
  return (
    a.ref?.path === b.ref.path &&
    a.ref.start === b.ref.start &&
    a.ref.end === b.ref.end &&
    a.text === b.text
  );
}
