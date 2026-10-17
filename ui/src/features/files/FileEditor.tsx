/**
 * File-tab edit controls and conflict banner. Depends on the lazy CodeMirror engine, files drafts,
 * and saveEditor; called by FileTab. Disk reads never replace a dirty buffer or its save fingerprint.
 */
import { Button } from "../../components/Controls";
import { lazy, Suspense, type ReactElement, useRef } from "react";
import type { FileContent } from "../../protocol/core";
import { useStore } from "../../state/createStore";
import type { Files } from "../../state/files";
import { useApp } from "../shell/AppContext";
import { CodeEditor, type EditorHandle } from "../editor/CodeEditor";
import { saveEditor } from "./saveEditor";

const MergeCompare = lazy(() =>
  import("../editor/MergeCompare").then((m) => ({ default: m.MergeCompare })),
);

/** Save/Cancel use the same draft actions as the Debug API. */
export function FileEditor({
  files,
  sessionId,
  content,
  line,
}: {
  files: Files;
  sessionId: string;
  content: FileContent;
  line: number | null;
}): ReactElement {
  const app = useApp();
  const draft = useStore(files.store, (s) => s.edits[sessionId]);
  const engine = useRef<EditorHandle | null>(null);
  if (draft === undefined) return <div className="empty">The editor is closed.</div>;
  const cancel = () => {
    if (!draft.saving) files.edit.cancel(sessionId);
  };
  const save = () => {
    void saveEditor(app, sessionId);
  };
  const reload = async () => {
    if (!(await files.actions.reloadEdit(sessionId)))
      files.edit.update(sessionId, {
        problem: "The new version could not be loaded as editable text.",
      });
  };
  return (
    <div className="file-edit" data-dirty={draft.text !== draft.base.text}>
      <div className="file-edit-bar" role="toolbar" aria-label="Edit the file">
        <span className="file-edit-hint">
          Editing {content.name}. <kbd>Ctrl</kbd> / <kbd>Cmd</kbd> <kbd>S</kbd> saves,{" "}
          <kbd>Esc</kbd> cancels.
        </span>
        <button
          data-testid="files.file-editor-find"
          type="button"
          className="mini"
          onClick={() => engine.current?.find()}
        >
          Find
        </button>
        <button
          data-testid="files.file-editor-soft-wrap"
          type="button"
          className="mini"
          aria-pressed={draft.wrap}
          onClick={() => {
            files.edit.update(sessionId, { wrap: !draft.wrap });
          }}
        >
          Soft wrap
        </button>
        <button
          data-testid="files.file-editor-save"
          type="button"
          className="mini primary"
          disabled={draft.saving}
          onClick={save}
        >
          {draft.saving ? "Saving…" : "Save"}
        </button>
        <button
          data-testid="files.file-editor-mini"
          type="button"
          className="mini"
          disabled={draft.saving}
          onClick={cancel}
        >
          Cancel
        </button>
      </div>
      {draft.conflict && (
        <div className="server-note file-edit-conflict" role="alert">
          <span>Changed on disk. Your edits are still here.</span>
          <Button
            data-testid="files.file-editor-reload"
            type="button"
            small
            variant="danger"
            disabled={draft.saving}
            onClick={() => void reload()}
          >
            Reload (lose my edits)
          </Button>
          <button
            data-testid="files.file-editor-keep-editing"
            type="button"
            className="mini"
            onClick={() => {
              files.edit.update(sessionId, { conflict: false });
              engine.current?.focus();
            }}
          >
            Keep editing
          </button>
          <button
            data-testid="files.file-editor-compare"
            type="button"
            className="mini"
            disabled={draft.disk.text === null || draft.disk.truncated}
            onClick={() => {
              files.edit.update(sessionId, { compare: true });
            }}
          >
            Compare mine vs disk
          </button>
        </div>
      )}
      {draft.problem !== null && (
        <p className="key-problem" role="alert">
          {draft.problem}
        </p>
      )}
      {draft.compare && draft.disk.text !== null ? (
        <Suspense fallback={<div className="empty">Loading comparison…</div>}>
          <MergeCompare
            mine={draft.text}
            disk={draft.disk.text}
            filename={content.name}
            onClose={() => {
              files.edit.update(sessionId, { compare: false });
            }}
          />
        </Suspense>
      ) : null}
      <div className={draft.compare ? "file-edit-engine hidden" : "file-edit-engine"}>
        <CodeEditor
          value={draft.text}
          onChange={(text) => {
            files.edit.update(sessionId, { text });
          }}
          label={`Text of ${content.name}`}
          filename={content.name}
          line={line}
          readOnly={draft.saving}
          wrap={draft.wrap}
          onSave={save}
          onCancel={cancel}
          onReady={(handle) => {
            engine.current = handle;
          }}
        />
      </div>
    </div>
  );
}
