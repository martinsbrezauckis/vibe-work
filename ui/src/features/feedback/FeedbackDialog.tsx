/** The two-step feedback panel; renders core-owned previews and facts, sends only after the separate Send action. */
import { useEffect, useRef } from "react";
import { Button } from "../../components/Controls";
import { Selector } from "../../components/Selector";
import { useModalFocus } from "../../hooks/useModalFocus";
import { useStore } from "../../state/createStore";
import { feedbackFor, type Feedback } from "../../state/feedback";
import { FEEDBACK_ENABLED } from "../../state/feedbackEnabled";
import { useApp } from "../shell/AppContext";
import "./feedback.css";

/** The panel bound to a feedback controller; also used by fixture and component tests. */
export function FeedbackPanel({ feedback }: { feedback: Feedback }) {
  const state = useStore(feedback.store, (s) => s);
  const ref = useRef<HTMLDivElement>(null);
  const open = state.view?.draft != null || state.sent;
  const close = () => {
    if (!state.busy) {
      feedback.dismissSent();
      void feedback.close();
    }
  };
  useModalFocus(open, ref, close);
  if (!open) return null;
  const { draft, view } = state;
  const reviewing = state.step === "review" && view?.review != null;
  const system = view?.review?.system ?? view?.system;
  return (
    <>
      <div className="scrim" data-testid="feedback.scrim" onClick={close} />
      <div
        className="modal feedback-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="feedback-title"
        ref={ref}
        tabIndex={-1}
        data-testid="feedback.dialog"
      >
        <header>
          <h2 id="feedback-title">
            {state.sent
              ? "Feedback sent"
              : reviewing
                ? "Send this report?"
                : "What would you like to report?"}
          </h2>
        </header>
        <section>
          {!state.sent && !reviewing && (
            <>
              <Selector
                testId="feedback.kind"
                label="Report type"
                value={draft.kind}
                options={[
                  { value: "bug", label: "Report a bug" },
                  { value: "feature", label: "Ask for a feature" },
                ]}
                disabled={state.busy}
                onChange={(kind) => {
                  feedback.edit({ ...draft, kind });
                }}
              />
              <label className="git-field">
                Summary
                <input
                  data-testid="feedback.title"
                  data-dialog-initial-focus="true"
                  value={draft.title}
                  maxLength={160}
                  disabled={state.busy}
                  onChange={(e) => {
                    feedback.edit({ ...draft, title: e.target.value });
                  }}
                />
              </label>
              <label className="git-field">
                {draft.kind === "bug" ? "What happened?" : "What would you like?"}
                <textarea
                  data-testid="feedback.description"
                  rows={6}
                  value={draft.description}
                  maxLength={8000}
                  disabled={state.busy}
                  placeholder={
                    draft.kind === "bug"
                      ? "Steps, expected result, actual result"
                      : "The task, the feature, an example"
                  }
                  onChange={(e) => {
                    feedback.edit({ ...draft, description: e.target.value });
                  }}
                />
              </label>
              <label className="git-field">
                Remarks
                <textarea
                  data-testid="feedback.remarks"
                  rows={2}
                  value={draft.remarks}
                  maxLength={2000}
                  disabled={state.busy}
                  onChange={(e) => {
                    feedback.edit({ ...draft, remarks: e.target.value });
                  }}
                />
              </label>
            </>
          )}
          {!state.sent && reviewing && (
            <>
              <h3>{view.review?.draft.title}</h3>
              <p className="feedback-prose">{view.review?.draft.description}</p>
              {view.review?.draft.remarks && (
                <>
                  <h4>Remarks</h4>
                  <p className="feedback-prose">{view.review.draft.remarks}</p>
                </>
              )}
              <p className="key-field-hint">Redacted preview</p>
              <label className="feedback-share">
                <input
                  type="checkbox"
                  data-testid="feedback.share"
                  checked={view.shareSystem}
                  disabled={state.busy}
                  onChange={(e) => void feedback.share(e.target.checked)}
                />
                Share system info
              </label>
              <dl className="feedback-facts" data-testid="feedback.system">
                <dt>Vibe Work</dt>
                <dd>{system?.appVersion}</dd>
                <dt>Vibe</dt>
                <dd>{system?.vibeVersion}</dd>
                <dt>System</dt>
                <dd>
                  {system?.os} {system?.osVersion}
                </dd>
                <dt>Architecture</dt>
                <dd>{system?.arch}</dd>
              </dl>
              {!view.available && <p role="status">Sending unavailable in this build</p>}
            </>
          )}
          {state.error !== null && (
            <p className="key-problem" role="alert">
              {state.error}
            </p>
          )}
        </section>
        <footer className="modal-actions">
          <Button data-testid="feedback.cancel" disabled={state.busy} onClick={close}>
            {state.sent ? "Close" : "Cancel"}
          </Button>
          {!state.sent && reviewing && (
            <Button
              data-testid="feedback.back"
              disabled={state.busy}
              onClick={() => {
                feedback.back();
              }}
            >
              Back
            </Button>
          )}
          {!state.sent && (
            <Button
              data-testid="feedback.submit"
              variant="primary"
              shape="outlined"
              disabled={
                state.busy ||
                (reviewing ? !view.available : !draft.title.trim() || !draft.description.trim())
              }
              onClick={() => void (reviewing ? feedback.send() : feedback.review())}
            >
              {state.busy
                ? state.sending
                  ? "Sending…"
                  : "Preparing…"
                : reviewing
                  ? "Send"
                  : "Review"}
            </Button>
          )}
        </footer>
      </div>
    </>
  );
}
function ConnectedFeedback({ feedback }: { feedback: Feedback }) {
  useEffect(() => feedback.start(), [feedback]);
  return <FeedbackPanel feedback={feedback} />;
}
/** App-level mount; agent drafts appear through core events, even when another session is selected. */
export function FeedbackDialog() {
  const { core } = useApp();
  return !FEEDBACK_ENABLED || core === null ? null : (
    <ConnectedFeedback feedback={feedbackFor(core)} />
  );
}
