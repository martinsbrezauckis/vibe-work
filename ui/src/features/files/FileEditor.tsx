/**
 * File-tab edit controls, the notice after a change on disk and the conflict banner for a disk version
 * that cannot be merged. Depends on the lazy CodeMirror engine, files drafts, DiskNotice and saveEditor; called by
 * FileTab. Disk reads never replace unsaved text without a merge the user can undo, nor its save fingerprint.
 */
import { Button } from "../../components/Controls";
import { lazy, Suspense, type ReactElement, useRef } from "react";
import type { FileContent } from "../../protocol/core";
import { createStore, useStore } from "../../state/createStore";
import type { FileWorkspace } from "../../state/fileWorkspace";
import type { FileReader } from "../../state/files";
import { useShellState } from "../shell/hooks";
import { formatKeys } from "../../state/shortcuts";
import { useApp } from "../shell/AppContext";
import { CodeEditor, type EditorHandle } from "../editor/CodeEditor";
import { DiskNotice } from "./DiskNotice";
import { registerLiveEditor } from "./liveEditors";
import { panelReading } from "./panelRead";
import { saveEditor } from "./saveEditor";

const MergeCompare = lazy(() =>
  import("../editor/MergeCompare").then((m) => ({ default: m.MergeCompare })),
);

/** Stands in for the workspace when the app has no files controller (never changes). */
const NO_WORKSPACE = createStore<Readonly<Record<string, FileWorkspace>>>({});

/** Save/Cancel use the same draft actions as the Debug API. */
export function FileEditor({
  files,
  sessionId,
  content,
  line,
  onReady,
  inWorkspace = false,
}: {
  files: FileReader;
  sessionId: string;
  content: FileContent;
  line: number | null;
  /** Lets the File tab read the live selection from its header action. */
  onReady?: (handle: EditorHandle | null) => void;
  /** The workspace keeps comparisons within its two editor slots. */
  inWorkspace?: boolean;
}): ReactElement {
  const app = useApp();
  const separate = useShellState((s) => s.openFilesIn === "tabs");
  const draft = useStore(files.store, (s) => s.edits[sessionId]);
  const markLines = useShellState((s) => s.markVibeLines);
  // The open-files mark says Vibe wrote this file (its file_edit or file_write finished).
  const byVibe = useStore(
    app.files?.workspace.store ?? NO_WORKSPACE,
    (s) => s[sessionId]?.documents[content.file]?.changed === true,
  );
  const engine = useRef<EditorHandle | null>(null);
  const unregister = useRef<(() => void) | null>(null);
  const reading = useStore(panelReading, (s) => s[sessionId] === true);
  // HTML and Markdown switch back to their preview, or show it beside the code.
  const previewed = content.kind === "markdown" || content.kind === "html";
  const besidePreview = useStore(
    app.files?.workspace.store ?? NO_WORKSPACE,
    (s) => s[sessionId]?.split?.right === `preview:${content.file}`,
  );
  if (draft === undefined) return <div className="empty">The editor is closed.</div>;
  const cancel = () => {
    if (!draft.saving) files.edit.cancel(sessionId);
  };
  const save = () => {
    void saveEditor(app, sessionId, "ui", files);
  };
  const compareDisk = () => {
    if (inWorkspace && draft.disk.text !== null && app.files) {
      app.files.workspace.activate(sessionId, content.file);
      app.files.workspace.compare(sessionId, draft.disk.text, "disk");
    } else files.edit.update(sessionId, { compare: true });
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
          Editing {content.name}. <kbd>{formatKeys("Mod+S")}</kbd> saves.
          {!separate && (
            <>
              {" "}
              <kbd>Esc</kbd> cancels.
            </>
          )}
        </span>
        <Button
          data-testid="files.file-editor-find"
          type="button"
          appearance="compact"
          onClick={() => engine.current?.find()}
        >
          Find
        </Button>
        {previewed && !besidePreview && (
          <Button
            data-testid="files.file-editor-preview"
            type="button"
            appearance="compact"
            disabled={draft.saving || draft.text !== draft.base.text}
            title={
              draft.text !== draft.base.text
                ? "Save first, or use Preview to the side to see your unsaved text"
                : undefined
            }
            onClick={cancel}
          >
            Preview
          </Button>
        )}
        {previewed && separate && !besidePreview && (
          <Button
            data-testid="files.file-editor-preview-side"
            type="button"
            appearance="compact"
            onClick={() => {
              const shown = app.files?.workspace.of(sessionId).documents[content.file]?.shown;
              app.files?.workspace.previewToSide(
                sessionId,
                shown ?? { file: content.file, display: content.display, line: null },
              );
            }}
          >
            Preview to the side
          </Button>
        )}
        <Button
          data-testid="files.file-editor-soft-wrap"
          type="button"
          appearance="compact"
          aria-pressed={draft.wrap}
          onClick={() => {
            files.edit.update(sessionId, { wrap: !draft.wrap });
          }}
        >
          Soft wrap
        </Button>
        <Button
          data-testid="files.file-editor-save"
          type="button"
          appearance="compact"
          variant="primary"
          disabled={draft.saving}
          onClick={save}
        >
          {draft.saving ? "Saving…" : "Save"}
        </Button>
        <Button
          data-testid="files.file-editor-mini"
          type="button"
          appearance="compact"
          disabled={draft.saving}
          onClick={cancel}
        >
          Cancel
        </Button>
      </div>
      <DiskNotice
        draft={draft}
        name={content.name}
        byVibe={byVibe}
        onChoose={(change) => {
          files.edit.apply(sessionId, change);
        }}
        onShowChanges={(target) => engine.current?.goToLine(target)}
        onCompare={compareDisk}
      />
      {draft.conflict && draft.notice?.kind !== "overlap" && (
        <div className="server-note file-edit-conflict" role="alert">
          <span>Changed on disk. Your edits are still here.</span>
          <Button
            data-testid="files.file-editor-reload"
            type="button"
            shape="outlined"
            variant="danger"
            disabled={draft.saving}
            onClick={() => void reload()}
          >
            Reload (lose my edits)
          </Button>
          <Button
            data-testid="files.file-editor-keep-editing"
            type="button"
            appearance="compact"
            onClick={() => {
              files.edit.update(sessionId, { conflict: false });
              engine.current?.focus();
            }}
          >
            Keep editing
          </Button>
          <Button
            data-testid="files.file-editor-compare"
            type="button"
            appearance="compact"
            disabled={draft.disk.text === null || draft.disk.truncated}
            onClick={compareDisk}
          >
            Compare mine vs disk
          </Button>
        </div>
      )}
      {reading && (
        <p className="file-edit-reading" role="status" data-testid="files.panel-reading">
          Vibe is reading this editor (open file and your selection).
        </p>
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
          marks={markLines ? draft.marks : []}
          onSave={save}
          onCancel={() => {
            if (app.shell.store.get().openFilesIn === "tabs") app.shell.actions.unfoldChat();
            else cancel();
          }}
          onReady={(handle) => {
            engine.current = handle;
            unregister.current?.();
            unregister.current =
              handle === null ? null : registerLiveEditor(sessionId, content.file, handle);
            onReady?.(handle);
          }}
        />
      </div>
    </div>
  );
}
