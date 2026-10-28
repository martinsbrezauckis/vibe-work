/** Renders one document or a focused editor pair. Uses the existing file view, reader and shared controls; called by FileTab. */
import { lazy, Suspense, useRef, type CSSProperties } from "react";
import { Button, IconButton } from "../../components/Controls";
import { useStore } from "../../state/createStore";
import { type Files } from "../../state/files";
import { EditorDivider } from "./EditorDivider";
import { SessionFile } from "./FileTab";
import { SidePreview } from "./SidePreview";
const SavedDocument = lazy(() =>
  import("../editor/SavedDocument").then((m) => ({ default: m.SavedDocument })),
);
const SavedComparison = lazy(() =>
  import("../editor/SavedComparison").then((m) => ({ default: m.SavedComparison })),
);
import { useShellState } from "../shell/hooks";
import { formatKeys } from "../../state/shortcuts";

/** Both views of one file share its buffer; opening a different file targets the focused side. */
export function EditorWorkspace({ files, sessionId }: { files: Files; sessionId: string }) {
  const view = useStore(files.workspace.store, (s) => s[sessionId]);
  useStore(files.store, (s) => s.tabs[sessionId]);
  const single = useShellState((s) => s.openFilesIn === "single");
  const root = useRef<HTMLDivElement>(null);
  if (!view?.active) return <SessionFile files={files} sessionId={sessionId} />;
  const pair = view.split;
  const active = view.documents[view.active];
  const reader = files.document(sessionId, active?.shown.file ?? "");
  const split = () => {
    if (active) void files.workspace.openToSide(sessionId, active.shown);
  };
  const pane = (file: string, side: "left" | "right") => {
    const doc = view.documents[file];
    const target = doc && files.document(sessionId, doc.shown.file);
    if (view.folded.includes(side))
      return (
        <div className="window-strip editor-folded-strip" data-side={side}>
          <IconButton
            data-testid="files.unfold-editor"
            data-item-id={side}
            icon="panel-left"
            label="Show editor"
            onClick={() => {
              files.workspace.fold(sessionId, side);
            }}
          />
          <span className="chat-strip-title">{doc?.shown.display}</span>
        </div>
      );
    return (
      <div
        data-testid="files.editor-pane"
        className="editor-pane"
        data-focused={pair?.focus === side}
        data-side={side}
        onFocusCapture={() => {
          files.workspace.focus(sessionId, side);
        }}
        onPointerDown={() => {
          files.workspace.focus(sessionId, side);
        }}
      >
        {target && doc.preview ? (
          <SidePreview
            reader={target}
            sessionId={sessionId}
            onClose={() => {
              files.workspace.close(sessionId, file);
            }}
          />
        ) : (
          target && (
            <SessionFile
              key={file}
              files={target}
              sessionId={sessionId}
              onFold={() => {
                files.workspace.fold(sessionId, side);
              }}
              focused={pair?.focus === side}
            />
          )
        )}
      </div>
    );
  };
  return (
    <div className="editor-workspace">
      <div className="editor-workspace-tools" role="toolbar" aria-label="Editor layout">
        <IconButton
          data-testid="files.editor-split"
          icon="panel-left"
          label="Open to the side"
          title={`Open to the side (${formatKeys("Mod+\\")})`}
          onClick={split}
        />
        {pair && (
          <Button
            data-testid="files.editor-restore"
            appearance="compact"
            onClick={() => {
              files.workspace.restore(sessionId);
            }}
          >
            Restore
          </Button>
        )}
        {reader && (
          <Button
            data-testid="files.editor-compare-saved"
            appearance="compact"
            disabled={reader.store.get().tabs[sessionId]?.content?.text == null}
            onClick={() => void files.workspace.compareSaved(sessionId)}
          >
            Compare with saved version
          </Button>
        )}
        {single && (
          <span className="editor-picker">
            <SessionFilePicker files={files} sessionId={sessionId} />
          </span>
        )}
      </div>
      {view.problem && (
        <p className="key-problem" role="alert">
          {view.problem}
        </p>
      )}
      {pair?.compare && reader ? (
        <Suspense
          fallback={
            <p className="key-line" role="status">
              Loading comparison…
            </p>
          }
        >
          <SavedComparison files={files} reader={reader} sessionId={sessionId} />
        </Suspense>
      ) : pair ? (
        <div
          className="editor-pair"
          ref={root}
          data-fold-left={view.folded.includes("left")}
          data-fold-right={view.folded.includes("right")}
          style={{ "--editor-ratio": `${String(view.ratio * 100)}%` } as CSSProperties}
        >
          {pane(pair.left, "left")}
          <EditorDivider files={files} sessionId={sessionId} ratio={view.ratio} container={root} />
          {pane(pair.right, "right")}
        </div>
      ) : active?.preview && reader ? (
        <SidePreview
          reader={reader}
          sessionId={sessionId}
          onClose={() => {
            files.workspace.close(sessionId, view.active ?? "");
          }}
        />
      ) : active?.saved !== undefined ? (
        <Suspense fallback={<p className="key-line">Loading saved version…</p>}>
          <SavedDocument
            files={files}
            sessionId={sessionId}
            file={view.active}
            shown={active.shown}
            text={active.saved}
            {...(active.savedName ? { savedName: active.savedName } : {})}
            {...(active.savedLabel ? { savedLabel: active.savedLabel } : {})}
          />
        </Suspense>
      ) : reader ? (
        <SessionFile key={view.active} files={reader} sessionId={sessionId} />
      ) : null}
    </div>
  );
}

import { Selector } from "../../components/Selector";
function SessionFilePicker({ files, sessionId }: { files: Files; sessionId: string }) {
  const view = useStore(files.workspace.store, (s) => s[sessionId]);
  if (!view?.active) return null;
  return (
    <Selector
      testId="files.open-file-picker"
      label="Recent files"
      value={view.active}
      options={view.order.map((file) => ({
        value: file,
        label: view.documents[file]?.shown.display ?? file,
      }))}
      onChange={(file) => {
        files.workspace.activate(sessionId, file);
      }}
    />
  );
}
