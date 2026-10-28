/**
 * The preview beside a file's code ("Preview to the side"): Markdown rendered from the
 * editor's text as it is typed (the safe Markdown module), HTML in the isolated viewer, which serves the file from
 * disk, so it reloads after each save and with Reload. Depends on: state/files (the file's reader), FileViews,
 * NativeViewport, Controls. Called by: EditorWorkspace (the right side of a preview pair, or the focused preview document).
 * Invariants: renders only; file content is never HTML in this window (the viewer webview shows HTML pages).
 */
import { type ReactElement, useEffect, useRef } from "react";
import { Button } from "../../components/Controls";
import { NativeViewport } from "../../components/NativeViewport";
import { useStore } from "../../state/createStore";
import { type FileReader, tabOf } from "../../state/files";
import { useApp } from "../shell/AppContext";
import { MarkdownView } from "./FileViews";

export function SidePreview({
  reader,
  sessionId,
  onClose,
}: {
  /** The reader of the file whose code is beside this preview. */
  reader: FileReader;
  sessionId: string;
  onClose: () => void;
}): ReactElement {
  const app = useApp();
  const tab = useStore(reader.store, (s) => tabOf(s, sessionId));
  const draft = useStore(reader.store, (s) => s.edits[sessionId]);
  const content = tab.content ?? draft?.base ?? null;
  const html = content?.kind === "html";
  const reload = () => {
    void app.core
      ?.run({ type: "files", request: { op: "reload", sessionId } })
      .catch(() => undefined);
  };
  // The viewer shows this file (another HTML tab may have used the session's viewer since).
  useEffect(() => {
    if (html) void reader.actions.setMode(sessionId, "rendered");
  }, [html, reader, sessionId]);
  // HTML: the viewer serves the file from disk, so a save (a new baseline) reloads it.
  const saved = draft?.base.fingerprint;
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    if (html && saved !== undefined) reload();
    // Only a new saved version reloads; `reload` itself is stable in effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [saved, html]);
  useEffect(
    () => () => {
      if (html) void reader.layout(null);
    },
    [html, reader],
  );
  return (
    <div className="file-tab side-preview" data-kind={content?.kind ?? "loading"}>
      <div className="file-head">
        <div className="file-title">
          <span className="file-name">{content?.name ?? tab.shown?.display}</span>
          <span className="file-meta">
            {html ? "Preview · reloads on Save" : "Preview · follows your typing"}
          </span>
        </div>
        <div className="file-tools">
          {html && (
            <Button
              data-testid="files.side-preview-reload"
              type="button"
              appearance="compact"
              onClick={reload}
            >
              Reload
            </Button>
          )}
          <Button
            data-testid="files.side-preview-close"
            type="button"
            appearance="compact"
            onClick={onClose}
          >
            Close preview
          </Button>
        </div>
      </div>
      <div className="file-body">
        {content === null ? (
          <div className="empty">Reading the file…</div>
        ) : html ? (
          <NativeViewport
            report={(rect) => void reader.layout(rect === null ? null : { sessionId, rect })}
            live
            label={`${content.name} (isolated view)`}
            className="file-viewport"
          />
        ) : (
          <MarkdownView
            content={{ ...content, text: draft?.text ?? content.text }}
            onOpenLink={(url) => {
              app.transcript.askLink(url);
            }}
          />
        )}
      </div>
    </div>
  );
}
