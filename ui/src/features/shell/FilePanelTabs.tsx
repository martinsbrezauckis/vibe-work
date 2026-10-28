/**
 * File tabs, the bounded visible strip, the all-open-files menu and each tab's right-click menu (right click, the
 * menu key or Shift F10: Close, Close other tabs, Close saved tabs, Open to the side or Back to one editor, Point
 * Vibe at this, Reveal in Files, Copy path, Show in folder). Uses shared tabs, RowMenu and the shared file
 * actions (files/fileMenu); called by SidePanel.
 */
import { createPortal } from "react-dom";
import { useState } from "react";
import { Tab, TabClose } from "../../components/Tabs";
import { Button } from "../../components/Controls";
import { Icon } from "../../components/Icon";
import { RowMenu, menuGroups, useRowMenu } from "../../components/RowMenu";
import {
  type FileTarget,
  copyText,
  openFileToSide,
  pointAtFile,
  revealInFiles,
  showInFolder,
} from "../files/fileMenu";
import { filesOf } from "../explorer/filesBridge";
import { useStore } from "../../state/createStore";
import { useApp } from "./AppContext";
import { useShellState } from "./hooks";
import type { Files } from "../../state/files";
import { absoluteFilePath } from "../files/model/context";
import { CloseFileDialog } from "../files/CloseFileDialog";
import { openFileMark, openFileName, savedWords } from "../files/openFiles";

/** Every file stays available in overflow; dirty files take precedence over saved tabs. */
export function FilePanelTabs({
  files,
  sessionId,
  active,
}: {
  files: Files;
  sessionId: string;
  active: boolean;
}) {
  const app = useApp();
  const view = useStore(files.workspace.store, (s) => s[sessionId]);
  useStore(files.store, (s) => s);
  const single = useShellState((s) => s.openFilesIn === "single");
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const tabMenu = useRowMenu<string>();
  const closing = view?.closing ?? null;
  if (!view) return null;
  const select = (file: string) => {
    files.workspace.activate(sessionId, file);
    app.shell.actions.openTab("file");
  };
  const close = (file: string) => {
    files.workspace.requestClose(sessionId, file);
  };
  const name = (file: string) => openFileName(files, sessionId, file);
  const fullPath = (file: string) => {
    const cwd = app.core?.store.get().sessions[sessionId]?.session["cwd"];
    const path =
      typeof cwd === "string"
        ? absoluteFilePath(view.documents[file]?.shown.display ?? file, cwd)
        : view.documents[file]?.shown.display;
    // The tab of a preview document carries a monitor icon instead of a word, so the name keeps its room.
    return view.documents[file]?.preview ? `${path ?? file} (preview)` : path;
  };
  const hidden = view.order.length - view.visible.length;
  const folder = () => {
    const cwd = app.core?.store.get().sessions[sessionId]?.session["cwd"];
    return typeof cwd === "string"
      ? cwd
      : sessionId.startsWith("files-workspace:")
        ? sessionId.slice("files-workspace:".length)
        : "";
  };
  const tabItems = (file: string) => {
    const doc = view.documents[file];
    if (doc === undefined) return [];
    const target: FileTarget = {
      owner: sessionId,
      folder: folder(),
      path: doc.shown.display,
      shown: doc.shown,
    };
    const side = doc.saved !== undefined || doc.preview === true;
    const inPair = view.split !== null && (view.split.left === file || view.split.right === file);
    const run = (action: () => unknown) => () => {
      void action();
    };
    return menuGroups(
      [
        {
          id: "close",
          label: "Close",
          keys: "Mod+W",
          run: () => {
            close(file);
          },
        },
        view.order.length > 1 && {
          id: "close-others",
          label: "Close other tabs",
          run: () => {
            const kept = view.order.filter(
              (f) => f !== file && !files.workspace.close(sessionId, f),
            );
            if (kept.length > 0) app.toasts.feedback("Tabs with unsaved edits stay open.");
          },
        },
        {
          id: "close-saved",
          label: "Close saved tabs",
          run: () => {
            files.workspace.closeSaved(sessionId);
          },
        },
      ],
      [
        inPair
          ? {
              id: "back",
              label: "Back to one editor",
              run: () => {
                if (view.split?.compare === true) files.workspace.close(sessionId, view.split.left);
                else files.workspace.restore(sessionId);
              },
            }
          : !side && {
              id: "open-side",
              label: "Open to the side",
              keys: "Mod+\\",
              run: run(() => openFileToSide(app, target)),
            },
        !side && {
          id: "point",
          label: "Point Vibe at this",
          keys: "Mod+L",
          run: run(() => pointAtFile(app, target)),
        },
      ],
      [
        filesOf(app) !== null && {
          id: "reveal-files",
          label: "Reveal in Files",
          run: () => {
            revealInFiles(app, target);
          },
        },
        {
          id: "copy-path",
          label: "Copy path",
          run: run(() =>
            copyText(
              app,
              fullPath(file)?.replace(/ \(preview\)$/, "") ?? doc.shown.display,
              "the path",
            ),
          ),
        },
        { id: "show-folder", label: "Show in folder", run: run(() => showInFolder(app, target)) },
      ],
    );
  };
  const mark = (file: string) => openFileMark(files, sessionId, file);
  return (
    <>
      {!single &&
        view.visible.map((file) => (
          <span
            key={file}
            className="ptab-group file-panel-tab"
            data-item-id={file}
            data-selected={active && view.active === file}
            title={fullPath(file)}
            {...tabMenu.triggers(file)}
          >
            <Tab
              data-testid="files.panel-tab"
              data-item-id={file}
              aria-selected={active && view.active === file}
              title={fullPath(file)}
              {...(view.documents[file]?.preview
                ? { "aria-label": `${name(file)} (preview)` }
                : {})}
              onClick={() => {
                select(file);
              }}
            >
              <Icon name={view.documents[file]?.preview ? "monitor" : "file"} size={14} />
              <span className="file-panel-name">{name(file)}</span>
              {view.documents[file]?.saved !== undefined && (
                <span className="file-saved-label">{savedWords(view.documents[file]).tab}</span>
              )}

              {files.workspace.dirty(sessionId, file) && (
                <span className="file-tab-dot" aria-label="Unsaved" title="Unsaved" />
              )}
              {view.documents[file]?.changed && (
                <span
                  className="file-tab-dot"
                  data-changed="true"
                  aria-label="Vibe changed it"
                  title="Vibe changed it"
                />
              )}
            </Tab>
            <TabClose
              data-testid="files.panel-tab-close"
              data-item-id={file}
              aria-label={`Close ${name(file)}`}
              title={`Close ${name(file)}`}
              onClick={() => {
                close(file);
              }}
            >
              <Icon name="x" size={12} />
            </TabClose>
          </span>
        ))}
      {!single && hidden > 0 && (
        <Button
          data-testid="files.open-tabs-overflow"
          appearance="compact"
          aria-haspopup="menu"
          aria-expanded={menu !== null}
          title={`${String(hidden)} more open files`}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setMenu({ x: r.left, y: r.bottom });
          }}
        >
          +{hidden}
        </Button>
      )}
      {menu &&
        createPortal(
          <RowMenu
            at={menu}
            label="Open files"
            heading={`${String(view.order.length)} open files`}
            testId="files.open-tabs-menu"
            onClose={() => {
              setMenu(null);
            }}
            items={[
              ...view.order.map((file) => ({
                id: file,
                name: `Open ${view.documents[file]?.shown.display ?? file}`,
                label: (
                  <span className="open-file-menu-row">
                    <span>
                      {name(file)}
                      {view.documents[file]?.saved !== undefined
                        ? ` · ${savedWords(view.documents[file]).tab}`
                        : view.documents[file]?.preview
                          ? " · preview"
                          : ""}
                      {mark(file) ? ` · ${mark(file) ?? ""}` : ""}
                    </span>
                    <span className="open-file-folder">{view.documents[file]?.shown.display}</span>
                  </span>
                ),
                run: () => {
                  select(file);
                },
              })),
              {
                id: "close-saved",
                label: "Close saved tabs",
                separated: true,
                run: () => {
                  files.workspace.closeSaved(sessionId);
                },
              },
            ]}
          />,
          document.body,
        )}
      {tabMenu.at &&
        createPortal(
          <RowMenu
            at={tabMenu.at}
            label={`Actions for ${view.documents[tabMenu.at.target]?.shown.display ?? tabMenu.at.target}`}
            heading={
              view.documents[tabMenu.at.target]?.saved !== undefined
                ? `${view.documents[tabMenu.at.target]?.shown.display ?? ""} · ${savedWords(view.documents[tabMenu.at.target]).tab}`
                : (view.documents[tabMenu.at.target]?.shown.display ?? tabMenu.at.target)
            }
            testId="files.tab-menu"
            items={tabItems(tabMenu.at.target)}
            onClose={tabMenu.close}
          />,
          document.body,
        )}
      {closing && (
        <CloseFileDialog
          files={files}
          sessionId={sessionId}
          file={closing}
          onClose={() => {
            files.workspace.cancelClose(sessionId);
          }}
        />
      )}
    </>
  );
}
