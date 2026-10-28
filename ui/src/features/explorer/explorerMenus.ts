/**
 * The Files tab's right-click menus: a file or folder row, a search hit (file name or text line), History's
 * "Changes since the last commit", a commit and a file inside a commit. Same grammar as every menu (RowMenu):
 * open, Point Vibe at this, versions, copy and show, destructive last; the header line names the target.
 * Depends on: files/fileMenu (shared file items and actions), the explorer store, the folder actions model,
 * AppContext. Called by: ExplorerView (one menu for the whole tab).
 */
import { type MenuItem, menuGroups } from "../../components/RowMenu";
import type { ExplorerVersion } from "../../protocol/core";
import { requestComposerFocus } from "../../state/composerFocus";
import { NEW_CHAT } from "../../state/newChatKey";
import {
  type FileTarget,
  compareWithCommit,
  copyText,
  fileMenuItems,
  openFile,
  openFileToSide,
  pointAtFile,
  pointChip,
  revealInFiles,
} from "../files/fileMenu";
import { absoluteFilePath } from "../files/model/context";
import type { App } from "../shell/AppContext";
import { TEXT } from "../../vocabulary";
import type { Explorer } from "./store";
import type { WorkspaceModel } from "./workspaceModel";

/** What a Files menu is for. */
export type ExplorerTarget =
  | { kind: "file"; path: string; place: "files" | "search" }
  | { kind: "folder"; path: string }
  | { kind: "hit"; path: string; line: number; text: string }
  | { kind: "pending" }
  | { kind: "version"; version: ExplorerVersion }
  | { kind: "versionFile"; version: ExplorerVersion; path: string };

/** What the menus need from the panel. */
export interface ExplorerMenuDeps {
  app: App;
  explorer: Explorer;
  /** Where files open (a session or the folder's New chat workspace). */
  owner: string;
  folder: string;
  /** A session is selected: Commit and Revert all live in its Changes tab. */
  session: boolean;
  workspace: WorkspaceModel;
  onReview: () => void;
}

/** A menu: the header line, the accessible label and the items. */
export interface BuiltMenu {
  heading: string;
  label: string;
  items: MenuItem[];
}

const plural = (n: number, one: string, many: string) =>
  `${n.toLocaleString("en-US")} ${n === 1 ? one : many}`;

function when(version: ExplorerVersion): string {
  return new Date(version.time * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** The menu for `target`. */
export function explorerMenu(deps: ExplorerMenuDeps, target: ExplorerTarget): BuiltMenu {
  const { app, explorer, owner, folder } = deps;
  const file = (path: string, line: number | null = null): FileTarget => ({
    owner,
    folder,
    path,
    line,
  });
  const run = (action: () => unknown) => () => {
    void action();
  };
  switch (target.kind) {
    case "file":
      return {
        heading: target.path,
        label: `Actions for ${target.path}`,
        items: fileMenuItems(app, file(target.path), target.place),
      };
    case "hit": {
      const at = `${target.path}:${String(target.line)}`;
      const line = file(target.path, target.line);
      return {
        heading: at,
        label: `Actions for ${at}`,
        items: menuGroups(
          [
            {
              id: "open",
              label: "Open at this line",
              keys: "Enter",
              run: run(() => openFile(app, line)),
            },
            {
              id: "open-side",
              label: "Open to the side",
              keys: "Mod+\\",
              run: run(() => openFileToSide(app, line)),
            },
          ],
          [
            {
              id: "point",
              label: "Point Vibe at this line",
              keys: "Mod+L",
              run: run(() =>
                pointAtFile(app, line, { start: target.line, end: target.line, text: target.text }),
              ),
            },
          ],
          [
            { id: "copy-link", label: "Copy line link", run: run(() => copyText(app, at)) },
            {
              id: "reveal-files",
              label: "Reveal in Files",
              run: run(() => {
                revealInFiles(app, line);
              }),
            },
          ],
        ),
      };
    }
    case "folder": {
      const open = explorer.store.get().expanded.includes(target.path);
      const path = absoluteFilePath(target.path, folder);
      return {
        heading: `${target.path}/`,
        label: `Actions for ${target.path}/`,
        items: menuGroups(
          [
            {
              id: "toggle",
              label: open ? "Close folder" : "Open folder",
              keys: open ? "←" : "→",
              run: () => {
                explorer.toggle(target.path);
              },
            },
            {
              id: "search-here",
              label: "Search in this folder",
              run: () => {
                explorer.searchIn(target.path);
                setTimeout(() => {
                  document
                    .querySelector<HTMLInputElement>('[data-testid="explorer.search"]')
                    ?.focus();
                }, 0);
              },
            },
            {
              id: "new-chat-here",
              label: `${TEXT.newChat} in this folder`,
              run: () => {
                app.shell.actions.selectSession(null);
                app.shell.actions.setNewFolder(path, "session");
                requestComposerFocus(NEW_CHAT);
              },
            },
            {
              id: "terminal-here",
              label: "Open a terminal here",
              run: () => {
                app.shell.actions.selectSession(null);
                app.shell.actions.setNewFolder(path, "session");
                app.shell.actions.openTab("terminal");
              },
            },
          ],
          [
            {
              id: "copy-path",
              label: "Copy path",
              run: run(() => copyText(app, path, "the path")),
            },
            app.sessions !== null && {
              id: "show-folder",
              label: "Show in folder",
              run: run(() => app.sessions?.revealPath(path)),
            },
          ],
          [
            explorer.store.get().expanded.length > 0 && {
              id: "close-all",
              label: "Close all folders",
              run: () => {
                explorer.collapseAll();
              },
            },
          ],
        ),
      };
    }
    case "pending": {
      const state = explorer.store.get();
      const paths = state.status?.files.map((f) => f.path) ?? [];
      const words = `Changes since the last commit · ${plural(paths.length, "file", "files")}`;
      const pending = (action: () => unknown) => () => {
        explorer.setPending(true);
        void action();
      };
      return {
        heading: words,
        label: `Actions for ${words}`,
        items: menuGroups(
          [
            {
              id: "show-files",
              label: "Show the changed files",
              keys: "Enter",
              run: () => {
                explorer.setPending(true);
              },
            },
            deps.session
              ? { id: "changes-tab", label: "Open the Changes tab", run: deps.onReview }
              : {
                  id: "review",
                  label: "Review all changes",
                  run: pending(() => deps.workspace.review()),
                },
            !deps.session && {
              id: "commit",
              label: "Commit…",
              run: pending(() => deps.workspace.commit()),
            },
          ],
          [
            paths.length > 0 && {
              id: "point",
              label: "Point Vibe at these changes",
              run: run(() =>
                pointChip(app, owner, folder, {
                  label: words,
                  source: "git",
                  text:
                    `Changes since the last commit in ${folder} (not committed yet), ` +
                    `${plural(paths.length, "file", "files")}: ${paths.slice(0, 50).join(", ")}` +
                    `${paths.length > 50 ? ", …" : ""}. See them with git status and git diff.`,
                }),
              ),
            },
          ],
          [
            !deps.session &&
              paths.length > 0 && {
                id: "revert-all",
                label: "Revert all changes…",
                danger: true,
                run: pending(() => deps.workspace.askRevert()),
              },
          ],
        ),
      };
    }
    case "version": {
      const v = target.version;
      return {
        heading: `Commit · ${when(v)}`,
        label: `Actions for the commit ${v.subject}`,
        items: menuGroups(
          [
            {
              id: "show-files",
              label: "Show its files",
              keys: "Enter",
              run: run(() => explorer.openVersion(v)),
            },
          ],
          [
            {
              id: "point",
              label: "Point Vibe at this commit",
              run: run(() =>
                pointChip(app, owner, folder, {
                  label: `Commit · ${v.subject}`,
                  source: "git",
                  text:
                    `The commit ${v.id} ("${v.subject}") by ${v.who}, ${when(v)}, in ${folder}. ` +
                    `See it with git show ${v.id.slice(0, 12)}.`,
                }),
              ),
            },
          ],
          [
            {
              id: "copy-id",
              label: "Copy the commit id",
              run: run(() => copyText(app, v.id, "the commit id")),
            },
          ],
        ),
      };
    }
    case "versionFile": {
      const v = target.version;
      const now = file(target.path);
      const name = target.path.split("/").at(-1) ?? target.path;
      return {
        heading: `${target.path} · ${when(v)}`,
        label: `Actions for ${target.path} in the commit ${v.subject}`,
        items: menuGroups(
          [
            {
              id: "compare-commit",
              label: "Open as it was in this commit",
              keys: "Enter",
              run: run(() => compareWithCommit(app, now, v.id)),
            },
            {
              id: "open-now",
              label: "Open the file as it is now",
              run: run(() => openFile(app, now)),
            },
          ],
          [
            {
              id: "point",
              label: "Point Vibe at this file in this commit",
              run: run(() =>
                pointChip(app, owner, folder, {
                  label: `${name} in “${v.subject}”`,
                  source: "git",
                  text:
                    `The file ${target.path} as it was in the commit ${v.id} ("${v.subject}"), in ${folder}. ` +
                    `Read it with git show ${v.id.slice(0, 12)}:./${target.path}.`,
                }),
              ),
            },
          ],
          [
            {
              id: "copy-path",
              label: "Copy path",
              run: run(() => copyText(app, absoluteFilePath(target.path, folder), "the path")),
            },
            {
              id: "reveal-files",
              label: "Reveal in Files",
              run: run(() => {
                revealInFiles(app, now);
              }),
            },
          ],
        ),
      };
    }
  }
}
