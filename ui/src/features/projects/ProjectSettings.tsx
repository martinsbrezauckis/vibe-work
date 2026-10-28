/**
 * One project's settings pane. Depends on projectInstructions, the shared DocEditor and shell
 * context; opened from project menus/details. Root AGENTS.md and docs/instructions Markdown
 * files are the only entries. Every text change is reviewed before saving; deletion asks natively.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../components/Controls";
import { IconButton } from "../../components/IconButton";
import { createStore, useStore } from "../../state/createStore";
import { projectInstructionsFor } from "../../state/projectInstructions";
import type { ProjectInstructions } from "../../state/projectInstructions";
import { DocEditorView } from "../settings/DocEditor";
import { useApp } from "../shell/AppContext";

const NO_TRUST_GUIDANCE = createStore<string | null>(null);

function Instructions({ controller, folder }: { controller: ProjectInstructions; folder: string }) {
  const { sessions } = useApp();
  const state = useStore(controller.store, (s) => s);
  const trust = state.trust;
  const guidance = useStore(sessions?.trustGuidance ?? NO_TRUST_GUIDANCE, (s) => s);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const busy = state.loading || state.view.busy !== "idle";
  useEffect(() => {
    void controller.refresh();
  }, [controller]);
  return (
    <section className="pj-dsec pi-instructions" aria-labelledby="pi-instructions-title">
      <div className="key-actions wrap">
        <h4 id="pi-instructions-title">Instructions</h4>
        <Button
          data-testid="projects.project-settings-set-adding"
          type="button"

          disabled={busy}
          onClick={() => {
            setAdding(true);
          }}
        >
          New file
        </Button>
        <Button
          data-testid="projects.project-settings-refresh"
          type="button"

          disabled={busy}
          onClick={() => void controller.refresh()}
        >
          Refresh files
        </Button>
      </div>
      <p className="key-line">
        Vibe loads AGENTS.md. Link other files from it so the agent knows to read them.
      </p>
      <p className="key-field-hint">
        Extra instruction files live in docs/instructions/. Other project Markdown is not listed.
      </p>
      <p className="key-field-hint" role="note" data-testid="project-untrusted">
        {trust === "untrusted"
          ? "Vibe will not read project instructions until the folder is trusted."
          : trust === "trusted"
            ? "This folder is trusted. New chats can read its project instructions."
            : "Folder trust could not be checked."}
      </p>
      {trust === "untrusted" && sessions !== null && (
        <div className="key-actions">
          <Button
            data-testid="projects.instructions-trust"
            disabled={busy}
            onClick={() => {
              void sessions.trust(folder).then(() => controller.refreshTrust());
            }}
          >
            Trust this folder
          </Button>
          {guidance !== null && (
            <>
              <p role="status">{guidance}</p>
              <Button
                data-testid="projects.instructions-choose"
                onClick={() => void sessions.pickFolder()}
              >
                Choose folder…
              </Button>
            </>
          )}
        </div>
      )}
      {state.loading && <p role="status">Reading instruction files…</p>}
      {state.error !== null && (
        <p className="key-problem" role="alert">
          {state.error}
        </p>
      )}
      <ul className="pi-files" aria-label="Instruction files">
        {state.files.map((file) => (
          <li data-item-id={file.name} key={file.name} data-instruction-file={file.name}>
            <div className="pi-file-name mono">{file.name}</div>
            <p className="key-field-hint">
              {file.name === "AGENTS.md"
                ? "Vibe reads this file when the folder is trusted"
                : file.linked
                  ? "Linked from AGENTS.md"
                  : "Not linked from AGENTS.md"}
              {!file.exists && " · Not created yet"}
              {!file.editable && " · Cannot edit this file"}
            </p>
            <div className="key-actions wrap">
              <Button
                data-testid="projects.project-settings-open"
                type="button"

                disabled={busy}
                onClick={() => void controller.open(file.name)}
              >
                {file.exists ? "Edit" : "Create AGENTS.md"}
              </Button>
              {file.name !== "AGENTS.md" && (
                <Button
                  data-testid="projects.project-settings-link"
                  type="button"

                  disabled={busy || !file.editable}
                  onClick={() => void controller.link(file.name, !file.linked)}
                >
                  {file.linked ? "Unlink from AGENTS.md" : "Link from AGENTS.md"}
                </Button>
              )}
              {file.exists && (
                <Button
                  data-testid="projects.project-settings-remove"
                  type="button"
                  shape="outlined"
                  variant="danger"
                  disabled={busy || !file.editable}
                  onClick={() => void controller.remove(file.name)}
                >
                  Delete
                </Button>
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
                data-testid="projects.project-settings-set-name"
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
            <Button
              data-testid="projects.project-settings-tb-btn"
              type="submit"

              disabled={busy || name === ""}
            >
              Create draft
            </Button>
            <Button
              data-testid="projects.project-settings-set-adding-2"
              type="button"

              disabled={busy}
              onClick={() => {
                setAdding(false);
              }}
            >
              Cancel
            </Button>
          </div>
        </form>
      )}
      {state.name !== null && (
        <DocEditorView scope="project" title={state.name} view={state.view} actions={controller} />
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
          <IconButton
            data-testid="projects.project-settings-x"
            ref={closeRef}
            icon="x"
            label="Close project settings"
            onClick={close}
          />
        </div>
        <p>{title}</p>
        {folder !== null && <p className="mono">{folder}</p>}
      </div>
      {controller === null ? (
        <div className="pj-dsec">
          <p className="key-line">
            Choose a main folder to manage this project's instruction files.
          </p>
          <Button
            data-testid="projects.project-settings-tb-btn-2"
            type="button"

            onClick={chooseFolder}
          >
            Choose main folder…
          </Button>
        </div>
      ) : (
        <Instructions key={folder} controller={controller} folder={folder ?? ""} />
      )}
    </aside>
  );
}
