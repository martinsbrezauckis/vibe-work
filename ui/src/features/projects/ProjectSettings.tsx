/**
 * One project's settings pane. Depends on projectInstructions, the shared DocEditor and shell
 * context; opened from project menus/details. Root AGENTS.md and docs/instructions Markdown
 * files are the only entries. Every text change is reviewed before saving; deletion asks natively.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { IconButton } from "../../components/IconButton";
import { useStore } from "../../state/createStore";
import { projectInstructionsFor } from "../../state/projectInstructions";
import type { ProjectInstructions } from "../../state/projectInstructions";
import { DocEditorView } from "../settings/DocEditor";
import { useApp } from "../shell/AppContext";

function Instructions({ controller, folder }: { controller: ProjectInstructions; folder: string }) {
  const { sessions } = useApp();
  const state = useStore(controller.store, (s) => s);
  const [trust, setTrust] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const busy = state.loading || state.view.busy !== "idle";
  useEffect(() => {
    void controller.refresh();
    let current = true;
    void sessions?.trustStatus(folder).then((result) => {
      if (current) setTrust(result?.status ?? null);
    });
    return () => {
      current = false;
    };
  }, [controller, folder, sessions]);
  return (
    <section className="pj-dsec pi-instructions" aria-labelledby="pi-instructions-title">
      <div className="key-actions wrap">
        <h4 id="pi-instructions-title">Instructions</h4>
        <button
          type="button"
          className="tb-btn"
          disabled={busy}
          onClick={() => {
            setAdding(true);
          }}
        >
          New file
        </button>
        <button
          type="button"
          className="tb-btn"
          disabled={busy}
          onClick={() => void controller.refresh()}
        >
          Refresh files
        </button>
      </div>
      <p className="key-line">
        Vibe loads AGENTS.md. Link other files from it so the agent knows to read them.
      </p>
      <p className="key-field-hint">
        Extra instruction files live in docs/instructions/. Other project Markdown is not listed.
      </p>
      <p className="key-field-hint" role="note" data-testid="project-untrusted">
        {trust === "untrusted"
          ? "This folder is not trusted. Vibe ignores project instructions until the folder is trusted."
          : trust === "trusted"
            ? "This folder is trusted. Vibe can read its project instructions."
            : "Vibe ignores project instructions until the folder is trusted."}
      </p>
      {state.loading && <p role="status">Reading instruction files...</p>}
      {state.error !== null && (
        <p className="key-problem" role="alert">
          {state.error}
        </p>
      )}
      <ul className="pi-files" aria-label="Instruction files">
        {state.files.map((file) => (
          <li key={file.name} data-instruction-file={file.name}>
            <div className="pi-file-name mono">{file.name}</div>
            <p className="key-field-hint">
              {file.name === "AGENTS.md"
                ? "Vibe reads this file"
                : file.linked
                  ? "Linked from AGENTS.md"
                  : "Not linked from AGENTS.md"}
              {!file.exists && " · Not created yet"}
              {!file.editable && " · Cannot edit this file"}
            </p>
            <div className="key-actions wrap">
              <button
                type="button"
                className="tb-btn"
                disabled={busy}
                onClick={() => void controller.open(file.name)}
              >
                {file.exists ? "Edit" : "Create AGENTS.md"}
              </button>
              {file.name !== "AGENTS.md" && (
                <button
                  type="button"
                  className="tb-btn"
                  disabled={busy || !file.editable}
                  onClick={() => void controller.link(file.name, !file.linked)}
                >
                  {file.linked ? "Unlink from AGENTS.md" : "Link from AGENTS.md"}
                </button>
              )}
              {file.exists && (
                <button
                  type="button"
                  className="tb-btn"
                  disabled={busy || !file.editable}
                  onClick={() => void controller.remove(file.name)}
                >
                  Delete
                </button>
              )}
            </div>
          </li>
        ))}
      </ul>
      {adding && (
        <form
          className="pi-new"
          onSubmit={(event) => {
            event.preventDefault();
            void controller.newFile(name).then((opened) => {
              if (opened) {
                setAdding(false);
                setName("");
              }
            });
          }}
        >
          <label className="key-field">
            File name
            <span className="key-field-row">
              <input
                autoFocus
                aria-label="File name"
                value={name}
                placeholder="code-style.md"
                maxLength={96}
                onChange={(event) => {
                  setName(event.target.value);
                }}
                disabled={busy}
              />
            </span>
          </label>
          <p className="key-field-hint">Created in docs/instructions/ after you review and Save.</p>
          <div className="key-actions">
            <button type="submit" className="tb-btn" disabled={busy || name === ""}>
              Create draft
            </button>
            <button
              type="button"
              className="tb-btn"
              disabled={busy}
              onClick={() => {
                setAdding(false);
              }}
            >
              Cancel
            </button>
          </div>
        </form>
      )}
      {state.name !== null && (
        <DocEditorView
          scope="project"
          title={state.name}
          blurb="Edit the text, review the diff, then Save. Nothing is written automatically."
          view={state.view}
          actions={controller}
        />
      )}
      {state.name === null && state.view.error !== null && (
        <p className="key-problem" role="alert">
          {state.view.error}
        </p>
      )}
    </section>
  );
}

/** Shows settings for a saved project, including the folderless state. */
export function ProjectSettings({
  projectId,
  title,
  folder,
  close,
  chooseFolder,
}: {
  projectId: string;
  title: string;
  folder: string | null;
  close: () => void;
  chooseFolder: () => void;
}) {
  const { core } = useApp();
  const closeRef = useRef<HTMLButtonElement>(null);
  const controller = useMemo(
    () =>
      core === null || folder === null ? null : projectInstructionsFor(core, projectId, folder),
    [core, projectId, folder],
  );
  useEffect(() => {
    closeRef.current?.focus();
  }, []);
  return (
    <aside
      className="pj-detail pi-settings"
      aria-label="Project settings"
      data-project-settings={projectId}
    >
      <div className="pj-dh">
        <div className="row">
          <h3>Project settings</h3>
          <IconButton ref={closeRef} icon="x" label="Close project settings" onClick={close} />
        </div>
        <p>{title}</p>
        {folder !== null && <p className="mono">{folder}</p>}
      </div>
      {controller === null ? (
        <div className="pj-dsec">
          <p className="key-line">
            Choose a main folder to manage this project's instruction files.
          </p>
          <button type="button" className="tb-btn" onClick={chooseFolder}>
            Choose main folder...
          </button>
        </div>
      ) : (
        <Instructions key={folder} controller={controller} folder={folder ?? ""} />
      )}
    </aside>
  );
}
