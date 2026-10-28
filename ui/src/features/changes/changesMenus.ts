/**
 * The Changes tab's right-click menus: a changed file (Open, Open to the side, Point Vibe at this, Keep,
 * Copy path, Show in folder, Revert last in red) and a changed block (Point Vibe at this change, Keep this
 * change, Add a line comment, Revert this change last in red). Each item runs the same store action as the
 * tab's own buttons. Depends on: the Changes store, line comments, files/fileMenu, RowMenu. Called by:
 * GitReview (file rows) and HunkBlock (blocks).
 */
import { type MenuItem, menuGroups } from "../../components/RowMenu";
import type { FileChange, Hunk } from "../../protocol/core";
import {
  type FileTarget,
  copyText,
  openFile,
  openFileToSide,
  pointAtFile,
  pointChip,
  showInFolder,
} from "../files/fileMenu";
import { absoluteFilePath } from "../files/model/context";
import type { App } from "../shell/AppContext";
import type { LineComments } from "./lineComments";
import { fileActions, hunkActions, hunkRange } from "./model";
import type { Changes } from "./store";

/** The file as the shared file actions see it (paths in Changes are relative to the repository). */
function targetOf(changes: Changes, sessionId: string, file: FileChange): FileTarget | null {
  const root = changes.store.get().views[sessionId]?.repoRoot;
  return root == null ? null : { owner: sessionId, folder: root, path: file.path };
}

/** The header line, label and items of a changed file's menu. */
export function changedFileMenu(
  app: App,
  changes: Changes,
  sessionId: string,
  file: FileChange,
  readOnly: boolean,
): { heading: string; label: string; items: MenuItem[] } {
  const target = targetOf(changes, sessionId, file);
  const acts = fileActions(file) && !readOnly;
  const run = (action: () => unknown) => () => {
    void action();
  };
  const deleted = file.status === "deleted";
  return {
    heading: file.path,
    label: `Actions for ${file.path}`,
    items: menuGroups(
      [
        target !== null &&
          !deleted && {
            id: "open",
            label: "Open",
            keys: "Enter",
            run: run(() => openFile(app, target)),
          },
        target !== null &&
          !deleted && {
            id: "open-side",
            label: "Open to the side",
            keys: "Mod+\\",
            run: run(() => openFileToSide(app, target)),
          },
      ],
      [
        target !== null && {
          id: "point",
          label: "Point Vibe at this",
          keys: "Mod+L",
          run: run(() => pointAtFile(app, target)),
        },
      ],
      [acts && { id: "keep", label: "Keep", run: run(() => changes.keepFile(sessionId, file)) }],
      [
        target !== null && {
          id: "copy-path",
          label: "Copy path",
          run: run(() => copyText(app, absoluteFilePath(target.path, target.folder), "the path")),
        },
        target !== null &&
          !deleted && {
            id: "show-folder",
            label: "Show in folder",
            run: run(() => showInFolder(app, target)),
          },
      ],
      [
        acts && {
          id: "revert",
          label: "Revert",
          danger: true,
          run: run(() => changes.revertFile(sessionId, file)),
        },
      ],
    ),
  };
}

/** The header line, label and items of a changed block's menu; `line` is the line index under the pointer. */
export function hunkMenu(
  app: App,
  changes: Changes,
  comments: LineComments | null,
  sessionId: string,
  file: FileChange,
  hunk: Hunk,
  line: number | null,
  act: (run: () => Promise<unknown>) => void,
  readOnly: boolean,
): { heading: string; label: string; items: MenuItem[] } {
  const range = hunkRange(hunk);
  const heading = `${file.path} · ${range}`;
  const acts = hunkActions(file) && !readOnly;
  const target = targetOf(changes, sessionId, file);
  const diff = hunk.lines
    .slice(0, 400)
    .map((l) => `${l.kind === "add" ? "+" : l.kind === "del" ? "-" : " "}${l.text}`)
    .join("\n");
  const first = hunk.lines.findIndex((l) => l.kind !== "ctx");
  return {
    heading,
    label: `Actions for ${heading}`,
    items: menuGroups(
      [
        target !== null && {
          id: "point",
          label: "Point Vibe at this change",
          run: () => {
            pointChip(app, sessionId, target.folder, {
              label: `Change · ${file.path} ${range}`,
              source: "git",
              text:
                `A change in ${file.path} (${range}) among this session's changes, not committed yet:\n` +
                `\`\`\`diff\n${diff}\n\`\`\`` +
                (hunk.lines.length > 400 ? "\n(cut: read the file for the rest)" : ""),
            });
          },
        },
      ],
      [
        acts && {
          id: "keep",
          label: "Keep this change",
          run: () => {
            act(() => changes.keepHunk(sessionId, file, hunk));
          },
        },
        comments !== null && {
          id: "comment",
          label: "Add a line comment",
          run: () => {
            comments.begin(
              sessionId,
              file.path,
              hunk.id,
              line ?? Math.max(0, first),
              hunk.lines.length,
              false,
            );
          },
        },
      ],
      [
        acts && {
          id: "revert",
          label: "Revert this change",
          danger: true,
          // Asks first (the tab's confirmation), like the block's Revert button.
          run: () => {
            void changes.revertHunk(sessionId, file, hunk);
          },
        },
      ],
    ),
  };
}
