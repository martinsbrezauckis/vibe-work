/**
 * One instruction editor: a text area for an `AGENTS.md`, the review step with the diff,
 * and the plain-words notes around it. Shared by personal and project settings.
 * Depends on: useInstructions, DiffPreview, state/instructions. Called by: InstructionsSettings, ProjectSettings.
 * Invariants: Save exists only in the review step, and writes exactly the text the diff shows; a file
 * the app may not edit (a link, a folder, not text) shows the reason and no editor; every failure is
 * shown as text; text a Debug API client wrote is labelled as such.
 */
import type { ReactNode } from "react";

import { type DocView, isDirty, reviewDiff } from "../../state/instructions";
import { DiffPreview } from "./DiffPreview";
import { useInstructionsController, useInstructionsState } from "./useInstructions";

interface Props {
  scope: "personal";
  title: string;
  /** Plain words under the title. */
  blurb: ReactNode;
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
  blurb: ReactNode;
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
      <p className="key-line">{blurb}</p>
      {doc !== null && (
        <p className="key-field-hint mono" data-testid={`${id}-path`}>
          {doc.path}
          {doc.exists ? "" : " (not created yet)"}
        </p>
      )}
      {doc?.note != null && <p className="key-field-hint">{doc.note}</p>}
      {editable && !view.reviewing && (
        <textarea
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
        <p className="key-field-hint" role="note" data-draft-by="debug-api">
          An external client (the Debug API) wrote text in this editor, not you. Read it before you
          save.
        </p>
      )}
      {view.reviewing && <DiffPreview lines={reviewDiff(view)} />}
      <div className="key-actions wrap">
        {editable && !view.reviewing && (
          <>
            <button
              type="button"
              className="tb-btn"
              disabled={!dirty || busy || view.stale}
              onClick={() => {
                actions.review();
              }}
            >
              Review changes
            </button>
            {dirty && (
              <button
                type="button"
                className="tb-btn"
                disabled={busy}
                onClick={() => {
                  actions.discard();
                }}
              >
                Discard changes
              </button>
            )}
            {extra}
          </>
        )}
        {view.reviewing && (
          <>
            <button
              type="button"
              className="tb-btn primary"
              disabled={busy}
              onClick={() => {
                void actions.save();
              }}
            >
              {view.busy === "saving" ? "Saving..." : "Save"}
            </button>
            <button
              type="button"
              className="tb-btn"
              disabled={busy}
              onClick={() => {
                actions.back();
              }}
            >
              Back to editing
            </button>
          </>
        )}
        {view.stale && (
          <button
            type="button"
            className="tb-btn"
            onClick={() => {
              void actions.reload();
            }}
          >
            Load the file again
          </button>
        )}
      </div>
      {view.notice !== null && (
        <p className="key-field-hint" role="status">
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
