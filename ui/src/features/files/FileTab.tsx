/**
 * The File tab: the file a click in the chat opened, for the selected session. Header (name, path,
 * kind, size, recent files, Rendered/Source, Reload, Show in folder, Open with default app), notices (still
 * loading, only the start of a big file, deleted, refused link), and the body per kind; HTML pages and PDFs show in
 * the isolated viewer placed over a `NativeViewport`. While the tab is visible the shown file is checked every
 * `POLL_MS` and re-read when it changed (live reload).
 * Depends on: state/files, FileViews, components/NativeViewport, model/kinds, shell hooks, AppContext.
 * Called by: register.tsx (the panel tab body).
 * Find (Ctrl F, `FileFindBar`: the current match is the line the text view shows) and Edit (`FileEditor`,
 * quick edits of a text file shown whole).
 * Invariants: renders only (decisions in state/files); file content is never HTML in this window.
 */
import { type ReactElement, useEffect, useState } from "react";

import { NativeViewport } from "../../components/NativeViewport";
import { Selector } from "../../components/Selector";
import type { FileContent, Rect } from "../../protocol/core";
import { useStore } from "../../state/createStore";
import { type FileTab as Tab, type Files, POLL_MS, tabOf, viewerKind } from "../../state/files";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";
import { FileEditor } from "./FileEditor";
import { FileFindBar, useFileFindLine } from "./FileFindBar";
import { BinaryView, ImageView, MarkdownView, TextView } from "./FileViews";
import { sizeText } from "./model/kinds";

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
      {tab.status === "gone" && (
        <p className="file-note warn" role="status">
          This file is not there any more
          {content === null ? "." : ". Showing the last version read."}
        </p>
      )}
      {tab.status === "error" && tab.error !== null && (
        <p className="file-note warn" role="alert">
          {tab.error}
        </p>
      )}
      {content?.truncated === true && (
        <p className="file-note" role="status">
          Showing the first 256 KB of {sizeText(content.size)}. Open with default app to see all of
          it.
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
    return <TextView content={content} line={findLine ?? tab.shown?.line ?? null} />;
  if (content.kind === "markdown")
    return <MarkdownView content={content} onOpenLink={onOpenLink} />;
  if (content.kind === "image") return <ImageView content={content} />;
  return <BinaryView content={content} />;
}

/** An unsaved edit: the file and content it started from, and the draft text so far. */
type KeptEdit = { file: string; display: string; content: FileContent; text: string };

/**
 * Unsaved edits per session, kept per app (its `files` instance) for as long as the window lives. The File tab is
 * remounted when another session is shown; without this its draft would be dropped by a session switch.
 * An entry is removed when the edit is saved or cancelled.
 */
const keptByApp = new WeakMap<Files, Map<string, KeptEdit>>();

function keptEditsOf(files: Files): Map<string, KeptEdit> {
  let kept = keptByApp.get(files);
  if (kept === undefined) {
    kept = new Map();
    keptByApp.set(files, kept);
  }
  return kept;
}

function SessionFile({ files, sessionId }: { files: Files; sessionId: string }): ReactElement {
  const app = useApp();
  const { transcript, toasts } = app;
  const tab = useStore(files.store, (s) => tabOf(s, sessionId));
  const keptEdits = keptEditsOf(files);
  const [busy, setBusy] = useState(false);
  // The edit in progress: the file and content it started from (the editor keeps its own draft and pinned base), and
  // whether the draft differs. Kept here so a file switch, a reload or a loading tab never unmounts the editor and
  // drops the draft: the editor stays on its own file until the user saves or cancels.
  const [edit, setEdit] = useState<{ file: string; display: string; content: FileContent } | null>(
    () => keptEdits.get(sessionId) ?? null,
  );
  const [dirty, setDirty] = useState(false);
  const editing = edit !== null;
  const content = tab.content;
  const findLine = useFileFindLine(sessionId, content?.text ?? null);
  const actions = files.actions;
  useEffect(() => {
    const timer = setInterval(() => void files.poll(sessionId), POLL_MS);
    return () => {
      clearInterval(timer);
      void files.layout(null);
    };
  }, [files, sessionId]);
  if (tab.shown === null)
    return <div className="empty">Click a file name in the chat to preview it here.</div>;
  const external = async (what: "open" | "reveal") => {
    setBusy(true);
    const error = await actions.external(sessionId, what);
    setBusy(false);
    if (error !== null) toasts.push({ kind: "warn", bellKind: "failure", text: error });
  };
  const canSource = content !== null && content.text !== null && content.kind !== "text";
  // Quick edits: a text file the tab read whole and still has.
  const canEdit =
    content !== null &&
    content.text !== null &&
    !content.truncated &&
    content.kind !== "image" &&
    content.kind !== "binary" &&
    tab.status === "ready";
  return (
    <div className="file-tab" data-status={tab.status} data-kind={content?.kind ?? "loading"}>
      <div className="file-head">
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
          {tab.recent.length > 1 && (
            <Selector
              label="Recent files"
              value={tab.shown.file}
              options={tab.recent.map((recent) => ({ value: recent.file, label: recent.display }))}
              disabled={dirty}
              {...(dirty ? { title: "Save or cancel your edit first" } : {})}
              onChange={(value) => {
                const next = tab.recent.find((r) => r.file === value);
                if (next !== undefined) void actions.show(sessionId, next);
              }}
            />
          )}
          {canSource && (
            <button
              type="button"
              className="mini"
              aria-pressed={tab.mode === "source"}
              onClick={() =>
                void actions.setMode(sessionId, tab.mode === "source" ? "rendered" : "source")
              }
            >
              {tab.mode === "source" ? "Rendered" : "Source"}
            </button>
          )}
          {canEdit && !editing && (
            <button
              type="button"
              className="mini"
              onClick={() => {
                const shown = tab.shown;
                if (shown !== null) {
                  const started = { file: shown.file, display: shown.display, content };
                  keptEdits.set(sessionId, { ...started, text: content.text ?? "" });
                  setEdit(started);
                }
              }}
            >
              Edit
            </button>
          )}
          {content?.text != null && (
            <button
              type="button"
              className="mini"
              aria-label="Find in this file"
              onClick={() => {
                app.find.actions.openFile(sessionId);
              }}
            >
              Find
            </button>
          )}
          <button type="button" className="mini" onClick={() => void actions.reload(sessionId)}>
            Reload
          </button>
          <button
            type="button"
            className="mini"
            disabled={busy}
            onClick={() => void external("reveal")}
          >
            Show in folder
          </button>
          {content !== null && content.openClass !== "refused" && (
            <button
              type="button"
              className="mini"
              disabled={busy}
              onClick={() => void external("open")}
            >
              Open with default app
            </button>
          )}
          <button
            type="button"
            className="mini"
            aria-label="Close the file"
            disabled={dirty}
            {...(dirty ? { title: "Save or cancel your edit first" } : {})}
            onClick={() => {
              actions.close(sessionId);
            }}
          >
            Close
          </button>
        </div>
      </div>
      <Notices tab={tab} content={content} />
      {tab.viewer?.external != null && viewerKind(content?.kind) && (
        <p className="file-note" role="status">
          The page tried to open <span className="file-url">{tab.viewer.external}</span>.{" "}
          <button
            type="button"
            className="mini"
            onClick={() => {
              transcript.askLink(tab.viewer?.external ?? "");
            }}
          >
            Open in browser…
          </button>
        </p>
      )}
      {content?.text != null && !editing && (
        <FileFindBar sessionId={sessionId} name={content.name} text={content.text} />
      )}
      <div className="file-body">
        {edit !== null ? (
          <>
            {edit.file !== tab.shown.file && (
              <p className="file-note warn" role="status">
                You are still editing {edit.content.name}. Save or cancel the edit to see{" "}
                {tab.shown.display}.
              </p>
            )}
            <FileEditor
              files={files}
              sessionId={sessionId}
              file={edit.file}
              display={edit.display}
              content={edit.content}
              line={edit.file === tab.shown.file ? (findLine ?? tab.shown.line) : null}
              initialText={keptEdits.get(sessionId)?.text}
              onText={(text) => {
                const kept = keptEdits.get(sessionId);
                if (kept !== undefined) keptEdits.set(sessionId, { ...kept, text });
              }}
              onDone={() => {
                keptEdits.delete(sessionId);
                setEdit(null);
                setDirty(false);
              }}
              onDirty={setDirty}
            />
          </>
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
  const sessionId = useShellState((s) => s.selected);
  if (sessionId === null) return <div className="empty">Choose a session first.</div>;
  return <SessionFile key={sessionId} files={files} sessionId={sessionId} />;
}
