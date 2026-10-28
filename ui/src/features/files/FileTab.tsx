/**
 * The File tab: the file a click in the chat opened, for the selected session. Header (name, path,
 * kind, size, recent files, Rendered/Source, Reload, Show in folder, Open with default app), notices (still
 * loading, only the start of a big file, deleted, refused link), and the body per kind; HTML pages and PDFs show in
 * the isolated viewer placed over a `NativeViewport`. While the tab is visible the shown file is checked every
 * `POLL_MS` and re-read when it changed (live reload).
 * Depends on: state/files, FileViews, components/NativeViewport, model/kinds, shell hooks, AppContext.
 * Called by: register.tsx (the panel tab body).
 * Find (Ctrl F, `FileFindBar`: the current match is the line the text view shows) and Edit (`FileEditor`,
 * quick edits of a text file shown whole). A right click on the text opens the code menu (`useCodeMenu`).
 * Invariants: renders only (decisions in state/files); file content is never HTML in this window.
 */
import { Button, IconButton } from "../../components/Controls";
import { type ReactElement, lazy, Suspense, useEffect, useRef, useState } from "react";

import { NativeViewport } from "../../components/NativeViewport";
import { Selector } from "../../components/Selector";
import type { FileContent, Rect } from "../../protocol/core";
import { useStore } from "../../state/createStore";
import { formatKeys } from "../../state/shortcuts";
import { canEditContent } from "../../state/fileEdits";
import {
  type FileTab as Tab,
  type FileReader,
  type Files,
  POLL_MS,
  tabOf,
  viewerKind,
} from "../../state/files";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";
const FileEditor = lazy(() => import("./FileEditor").then((m) => ({ default: m.FileEditor })));
import { FileFindBar, useFileFindLine } from "./FileFindBar";
import { BinaryView, ImageView, MarkdownView, TextView } from "./FileViews";
import { sizeText } from "./model/kinds";
import type { EditorHandle } from "../editor/CodeEditor";
import { EditorWorkspace } from "./EditorWorkspace";
import { canPointFile, pointFile, POINT_FILE_KEYS } from "./point";
import { useCodeMenu } from "./useCodeMenu";

/** A text file the tab read whole and still has (the same rule as Edit). */
function editable(content: FileContent | null, status: Tab["status"]): boolean {
  return content !== null && canEditContent(content) && status === "ready";
}

const KIND_LABEL: Record<FileContent["kind"], string> = {
  text: "Text",
  markdown: "Markdown",
  html: "HTML page",
  image: "Picture",
  pdf: "PDF",
  binary: "File",
};

function Notices({ tab, content }: { tab: Tab; content: FileContent | null }): ReactElement {
  return (
    <>
      {tab.openNote != null && (
        <p className="file-note" role="status">
          {tab.openNote}
        </p>
      )}
      {tab.status === "gone" && (
        <p className="server-note" role="status">
          This file is not there any more
          {content === null ? "." : ". Showing the last version read."}
        </p>
      )}
      {tab.status === "error" && tab.error !== null && (
        <p className="key-problem" role="alert">
          {tab.error}
        </p>
      )}
      {content?.truncated === true && (
        <p className="file-note" role="status">
          Showing the first 256 KB of {sizeText(content.size)}. Open with default app to see all of
          it.
        </p>
      )}
      {tab.editNotice !== null && (
        <p className="file-note" role="status">
          {tab.editNotice}
        </p>
      )}
    </>
  );
}

function Body({
  tab,
  content,
  findLine,
  report,
  onOpenLink,
}: {
  tab: Tab;
  content: FileContent;
  /** The find bar's current line (shows the text with line numbers). */
  findLine: number | null;
  report: (rect: Rect | null) => void;
  onOpenLink: (url: string) => void;
}): ReactElement {
  const source = (tab.mode === "source" || findLine !== null) && content.text !== null;
  if (viewerKind(content.kind) && !source)
    return (
      <NativeViewport
        report={report}
        live
        label={`${content.name} (isolated view)`}
        className="file-viewport"
      />
    );
  if (source || content.kind === "text" || (content.kind === "html" && content.text !== null))
    return (
      <TextView
        content={content}
        line={findLine ?? tab.shown?.line ?? null}
        {...(findLine !== null || tab.shown?.end === undefined ? {} : { end: tab.shown.end })}
      />
    );
  if (content.kind === "markdown")
    return <MarkdownView content={content} onOpenLink={onOpenLink} />;
  if (content.kind === "image") return <ImageView content={content} />;
  return <BinaryView content={content} />;
}

export function SessionFile({
  files,
  sessionId,
  focused = true,
  onFold,
}: {
  files: FileReader;
  sessionId: string;
  focused?: boolean;
  onFold?: () => void;
}): ReactElement {
  const app = useApp();
  const { transcript, toasts } = app;
  const separate = useShellState((s) => s.openFilesIn === "tabs");
  const tab = useStore(files.store, (s) => tabOf(s, sessionId));
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const engine = useRef<EditorHandle | null>(null);
  const draft = useStore(files.store, (s) => s.edits[sessionId]);
  const editing = draft !== undefined;
  const content = tab.content;
  const canPoint = canPointFile(draft?.base ?? content, tab.mode === "source", editing);
  const pointLabel = `Point Vibe at this: add the selected lines (or this file) to your next message (${formatKeys(POINT_FILE_KEYS).replace("Cmd ", "⌘").replaceAll(" ", "+")})`;
  const findLine = useFileFindLine(sessionId, content?.text ?? null);
  const actions = files.actions;
  const point = () => {
    setNotice(pointFile(app, sessionId, root.current, engine.current));
  };
  // Right click, the menu key or Shift F10 on the text: the code menu.
  const codeMenu = useCodeMenu(app, files, sessionId, root, engine);
  useEffect(() => {
    if (!canPoint || !focused) return;
    if (app.files?.workspace.of(sessionId).active !== tab.shown?.file) return;
    const run = () => {
      setNotice(pointFile(app, sessionId, root.current, engine.current));
    };
    const removeCommand = app.commands.register({
      id: "files.point",
      label: "Point Vibe at this (File tab)",
      section: "Files",
      keys: POINT_FILE_KEYS,
      run,
    });
    return () => {
      removeCommand();
    };
  }, [app, sessionId, canPoint, tab.shown?.file, focused]);
  useEffect(() => {
    const timer = setInterval(() => void files.poll(sessionId), POLL_MS);
    return () => {
      clearInterval(timer);
      void files.layout(null);
    };
  }, [files, sessionId]);
  if (tab.shown === null)
    return (
      <>
        <Notices tab={tab} content={content} />
        <div data-item-id={sessionId} className="empty">
          Click a file name in the chat to open it here. Right-click it to edit it.
        </div>
      </>
    );
  const external = async (what: "open" | "reveal") => {
    setBusy(true);
    const error = await actions.external(sessionId, what);
    setBusy(false);
    if (error !== null) toasts.push({ kind: "warn", bellKind: "failure", text: error });
  };
  const canSource = content !== null && content.text !== null && content.kind !== "text";
  // HTML and Markdown show their code on the left and the preview on the right (Separate tabs).
  const canPreviewSide =
    separate &&
    editable(content, tab.status) &&
    (content?.kind === "markdown" || content?.kind === "html");
  // Quick edits: a text file the tab read whole and still has.
  const canEdit = editable(content, tab.status);
  return (
    <div
      ref={root}
      data-item-id={sessionId}
      className="file-tab"
      data-status={tab.status}
      data-kind={content?.kind ?? "loading"}
    >
      <div className="file-head">
        {onFold && (
          <IconButton
            data-testid="files.fold-editor"
            data-item-id={tab.shown.file}
            icon="panel-left"
            label="Fold editor"
            onClick={onFold}
          />
        )}
        <div className="file-title">
          <span className="file-name">{content?.name ?? tab.shown.display}</span>
          <span className="file-path" title={tab.shown.display}>
            {tab.shown.display}
          </span>
          {content !== null && (
            <span className="file-meta">
              {KIND_LABEL[content.kind]} · {sizeText(content.size)}
            </span>
          )}
        </div>
        <div className="file-tools">
          {!separate && tab.recent.length > 1 && (
            <Selector
              testId="files.file-tab-find"
              label="Recent files"
              disabled={editing}
              value={tab.shown.file}
              options={tab.recent.map((recent) => ({ value: recent.file, label: recent.display }))}
              onChange={(value) => {
                const next = tab.recent.find((r) => r.file === value);
                if (next !== undefined) void actions.show(sessionId, next);
              }}
            />
          )}
          {canSource && !editing && (
            <Button
              data-testid="files.file-tab-set-mode"
              type="button"
              appearance="compact"
              aria-pressed={tab.mode === "source"}
              onClick={() =>
                void actions.setMode(sessionId, tab.mode === "source" ? "rendered" : "source")
              }
            >
              {tab.mode === "source" ? "Rendered" : "Source"}
            </Button>
          )}
          {canEdit && !editing && (
            <Button
              data-testid="files.file-tab-set"
              type="button"
              appearance="compact"
              onClick={() => {
                files.edit.begin(sessionId);
              }}
            >
              Edit
            </Button>
          )}
          {canPreviewSide && !editing && (
            <Button
              data-testid="files.file-tab-preview-side"
              type="button"
              appearance="compact"
              onClick={() => {
                if (tab.shown) app.files?.workspace.previewToSide(sessionId, tab.shown);
              }}
            >
              Preview to the side
            </Button>
          )}
          {content?.text != null && !editing && (
            <Button
              data-testid="files.file-tab-open-file"
              type="button"
              appearance="compact"
              aria-label="Find in this file"
              onClick={() => {
                app.find.actions.openFile(sessionId);
              }}
            >
              Find
            </Button>
          )}
          {canPoint && (
            <Button
              data-testid="files.file-tab-point"
              type="button"
              appearance="compact"
              title={pointLabel}
              aria-label={pointLabel}
              onMouseDown={(event) => {
                event.preventDefault();
              }}
              onClick={point}
            >
              Point Vibe at this
            </Button>
          )}
          <Button
            data-testid="files.file-tab-reload"
            type="button"
            appearance="compact"
            onClick={() => void actions.reload(sessionId)}
          >
            Reload
          </Button>
          <Button
            data-testid="files.file-tab-external"
            type="button"
            appearance="compact"
            disabled={busy}
            onClick={() => void external("reveal")}
          >
            Show in folder
          </Button>
          {content !== null && content.openClass !== "refused" && (
            <Button
              data-testid="files.file-tab-external-2"
              type="button"
              appearance="compact"
              disabled={busy}
              onClick={() => void external("open")}
            >
              Open with default app
            </Button>
          )}
          <Button
            data-testid="files.file-tab-close"
            type="button"
            appearance="compact"
            aria-label="Close the file"
            disabled={
              draft?.saving === true || (draft !== undefined && draft.text !== draft.base.text)
            }
            title={editing ? "Save or Cancel your edit before closing the file" : undefined}
            onClick={() => {
              actions.close(sessionId);
            }}
          >
            Close
          </Button>
        </div>
      </div>
      <Notices tab={tab} content={content} />
      {notice !== null && (
        <p className="file-note" role="status">
          {notice}
        </p>
      )}
      {tab.viewer?.external != null && viewerKind(content?.kind) && (
        <p className="file-note" role="status">
          The page tried to open <span className="file-url">{tab.viewer.external}</span>.{" "}
          <Button
            data-testid="files.file-tab-ask-link"
            type="button"
            appearance="compact"
            onClick={() => {
              transcript.askLink(tab.viewer?.external ?? "");
            }}
          >
            Open in browser…
          </Button>
        </p>
      )}
      {content?.text != null && !editing && (
        <FileFindBar sessionId={sessionId} name={content.name} text={content.text} />
      )}
      <div className="file-body" {...codeMenu.triggers}>
        {codeMenu.element}
        {draft !== undefined ? (
          <Suspense fallback={<div className="empty">Loading editor…</div>}>
            {draft.base.file !== tab.shown.file && (
              <p className="server-note" role="status">
                You are still editing {draft.base.name}. Save or cancel the edit to see{" "}
                {tab.shown.display}.
              </p>
            )}
            <FileEditor
              files={files}
              inWorkspace
              sessionId={sessionId}
              content={draft.base}
              line={draft.base.file === tab.shown.file ? (findLine ?? tab.shown.line) : null}
              onReady={(handle) => {
                engine.current = handle;
              }}
            />
          </Suspense>
        ) : content === null ? (
          <div className="empty">{tab.status === "loading" ? "Reading the file…" : ""}</div>
        ) : (
          <Body
            tab={tab}
            content={content}
            findLine={findLine}
            report={(rect) => void files.layout(rect === null ? null : { sessionId, rect })}
            onOpenLink={(url) => {
              transcript.askLink(url);
            }}
          />
        )}
      </div>
    </div>
  );
}

/** The tab body: the selected session's file. */
export function FileTab({ files }: { files: Files }): ReactElement {
  const sessionId = useShellState(
    (s) => s.selected ?? (s.newFolder ? `files-workspace:${s.newFolder}` : null),
  );
  if (sessionId === null) return <div className="empty">Choose a session first.</div>;
  return <EditorWorkspace key={sessionId} files={files} sessionId={sessionId} />;
}
