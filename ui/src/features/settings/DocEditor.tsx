/**
 * One instruction editor: an `AGENTS.md` file shown as the file (its name and path), its text and Save. Shared by
 * personal and project settings.
 * Depends on: useInstructions, DiffPreview, state/instructions. Called by: InstructionsSettings, ProjectSettings.
 * Invariants: the person's Save writes the editor's text at once; text a Debug API client wrote is labelled and saved
 * only from the review step, which shows the diff of exactly what Save writes; a file the app may not edit (a link,
 * a folder, not text) shows the reason and no editor; every failure is shown as text.
 */
import { Button } from "../../components/Controls";
import { PathLine } from "../../components/PathLine";
import type { ReactNode } from "react";

import { type DocView, isDirty, reviewDiff } from "../../state/instructions";
import { DiffPreview } from "./DiffPreview";
import { useInstructionsController, useInstructionsState } from "./useInstructions";

interface Props {
  scope: "personal";
  title: string;
  /** Plain words under the title, if any. */
  blurb?: ReactNode;
  /** Extra controls next to the editor's buttons (the CLI copy button). */
  extra?: ReactNode;
}

/** The editor. */
export function DocEditor({ scope, title, blurb, extra }: Props) {
  const controller = useInstructionsController();
  const view = useInstructionsState((s) => s[scope]);
  if (controller === null) return null;
  return (
    <DocEditorView
      scope={scope}
      title={title}
      blurb={blurb}
      extra={extra}
      view={view}
      actions={{
        setDraft: (text) => {
          controller.setDraft(scope, text);
        },
        review: () => {
          controller.review(scope);
        },
        discard: () => {
          controller.discard(scope);
        },
        save: () => controller.save(scope),
        back: () => {
          controller.back(scope);
        },
        reload: () => controller.load(scope, true),
      }}
    />
  );
}

/** Pure shared editor: the caller owns document loading and reviewed saves. */
export function DocEditorView({
  scope,
  title,
  blurb,
  extra,
  view,
  actions,
}: {
  scope: string;
  title: string;
  blurb?: ReactNode;
  extra?: ReactNode;
  view: DocView;
  actions: {
    setDraft: (text: string) => void;
    review: () => void;
    discard: () => void;
    save: () => Promise<void>;
    back: () => void;
    reload: () => Promise<void>;
  };
}) {
  const doc = view.doc;
  const dirty = isDirty(view);
  const editable = doc !== null && doc.editable;
  const busy = view.busy !== "idle";
  const id = `instructions-${scope}`;

  return (
    <div className="instr" data-scope={scope}>
      <h4 id={`${id}-title`}>{title}</h4>
      {blurb !== undefined && <p className="key-line">{blurb}</p>}
      {doc !== null && (
        <div className="key-field-hint ui-help" data-testid={`${id}-path`}>
          <PathLine path={doc.path} />
          {/* The personal file can hold only Vibe Work's managed note, which the editor does not show. */}
          {doc.exists || scope === "personal" ? null : <span>Not created yet.</span>}
        </div>
      )}
      {doc?.note != null && <p className="key-field-hint ui-help">{doc.note}</p>}
      {editable && !view.reviewing && (
        <textarea
          data-testid="settings.doc-editor-set-draft"
          id={id}
          className="instr-text"
          aria-labelledby={`${id}-title`}
          rows={7}
          spellCheck={false}
          value={view.draft}
          disabled={busy}
          onChange={(e) => {
            actions.setDraft(e.target.value);
          }}
        />
      )}
      {view.draftBy === "debug-api" && (
        <p className="key-field-hint ui-help" role="note" data-draft-by="debug-api">
          An external client (the Debug API) wrote text in this editor, not you. Read it before you
          save.
        </p>
      )}
      {view.reviewing && <DiffPreview lines={reviewDiff(view)} />}
      <div className="set-actions">
        {editable && !view.reviewing && (
          <>
            {view.draftBy === "debug-api" ? (
              <Button
                data-testid="settings.doc-editor-review"
                type="button"
                shape="outlined"
                disabled={!dirty || busy || view.stale}
                onClick={() => {
                  actions.review();
                }}
              >
                Review changes
              </Button>
            ) : (
              <Button
                data-testid="settings.doc-editor-save-now"
                type="button"
                shape="outlined"
                variant="primary"
                disabled={!dirty || busy || view.stale}
                onClick={() => {
                  void actions.save();
                }}
              >
                {view.busy === "saving" ? "Saving…" : "Save"}
              </Button>
            )}
            {dirty && (
              <Button
                variant="danger"
                data-testid="settings.doc-editor-discard"
                type="button"
                shape="outlined"
                disabled={busy}
                onClick={() => {
                  actions.discard();
                }}
              >
                Discard changes
              </Button>
            )}
            {extra}
          </>
        )}
        {view.reviewing && (
          <>
            <Button
              data-testid="settings.doc-editor-save"
              type="button"
              shape="outlined"
              variant="primary"
              disabled={busy}
              onClick={() => {
                void actions.save();
              }}
            >
              {view.busy === "saving" ? "Saving…" : "Save"}
            </Button>
            <Button
              data-testid="settings.doc-editor-back"
              type="button"
              shape="outlined"
              disabled={busy}
              onClick={() => {
                actions.back();
              }}
            >
              Back to editing
            </Button>
          </>
        )}
        {view.stale && (
          <Button
            data-testid="settings.doc-editor-reload"
            type="button"
            shape="outlined"
            onClick={() => {
              void actions.reload();
            }}
          >
            Load the file again
          </Button>
        )}
      </div>
      {view.notice !== null && (
        <p className="key-field-hint ui-help" role="status">
          {view.notice}
        </p>
      )}
      {view.error !== null && (
        <p className="key-problem" role="alert">
          {view.error}
        </p>
      )}
    </div>
  );
}
