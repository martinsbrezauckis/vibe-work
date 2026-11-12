/** Single feedback form; renders core-owned facts and a secret-safe screenshot, sends only on the person's click. */
import { useEffect, useRef } from "react";
import { Button } from "../../components/Controls";
import { Tab, Tabs } from "../../components/Tabs";
import { useModalFocus } from "../../hooks/useModalFocus";
import { useStore } from "../../state/createStore";
import { feedbackFor, type Feedback } from "../../state/feedback";
import type { CoreController } from "../../state/coreStore";
import { useApp } from "../shell/AppContext";

/** The panel bound to a feedback controller; also used by fixture and component tests. */
export function FeedbackPanel({ feedback }: { feedback: Feedback }) {
  const state = useStore(feedback.store, (s) => s);
  const ref = useRef<HTMLDivElement>(null);
  const previewBack = useRef<HTMLButtonElement>(null);
  const thumbnail = useRef<HTMLButtonElement>(null);
  const wasPreviewing = useRef(false);
  useEffect(() => {
    if (state.previewing) previewBack.current?.focus();
    else if (wasPreviewing.current) thumbnail.current?.focus();
    wasPreviewing.current = state.previewing;
  }, [state.previewing]);
  const open = state.view?.draft != null || state.sent;
  const discard = () => {
    if (!state.busy) {
      feedback.dismissSent();
      void feedback.close();
    }
  };
  const close = () => {
    if (state.busy) return;
    if (state.previewing) feedback.preview(false);
    else discard();
  };
  useModalFocus(open, ref, close);
  if (!open) return null;
  const { draft, view } = state;
  const system = view?.review?.system ?? view?.system;
  const screenshot = view?.screenshot;
  const image = screenshot
    ? `data:${screenshot.mimeType};base64,${screenshot.dataBase64}`
    : undefined;
  return (
    <div className={state.capturing ? "feedback-capturing" : undefined}>
      <div className="scrim" data-testid="feedback.scrim" onClick={discard} />
      <div
        className="modal feedback-dialog"
        data-image-preview={state.previewing}
        role="dialog"
        aria-modal={state.capturing ? undefined : true}
        aria-labelledby="feedback-title"
        ref={ref}
        tabIndex={-1}
        data-testid="feedback.dialog"
      >
        <header>
          <h2 id="feedback-title">
            {state.sent
              ? "Feedback sent"
              : state.previewing
                ? "Vibe Work screenshot"
                : "Send feedback"}
          </h2>
        </header>
        {state.sent ? (
          state.reportId ? (
            <section>
              <p className="key-line">Reference {state.reportId}</p>
            </section>
          ) : null
        ) : (
          <section>
            {state.previewing && image && (
              <>
                <Button
                  ref={previewBack}
                  data-testid="feedback.preview-back"
                  onClick={() => {
                    feedback.preview(false);
                  }}
                >
                  Back to form
                </Button>
                <img className="feedback-image" src={image} alt="Vibe Work screenshot" />
              </>
            )}
            {!state.previewing && (
              <>
                <Tabs variant="segmented" aria-label="Feedback type" data-testid="feedback.kind">
                  {(
                    [
                      ["bug", "Bug"],
                      ["feature", "Suggestion"],
                    ] as const
                  ).map(([kind, label]) => (
                    <Tab
                      key={kind}
                      data-testid="feedback.kind-option"
                      data-item-id={kind}
                      aria-selected={draft.kind === kind}
                      disabled={state.busy}
                      onClick={() => {
                        feedback.edit({ ...draft, kind });
                      }}
                    >
                      {label}
                    </Tab>
                  ))}
                </Tabs>
                <label className="git-field">
                  Title
                  <input
                    aria-label="Title"
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
                  Description
                  <textarea
                    aria-label="Description"
                    data-testid="feedback.description"
                    rows={5}
                    value={draft.description}
                    maxLength={8000}
                    disabled={state.busy}
                    placeholder={
                      draft.kind === "bug"
                        ? "What happened? What did you expect?"
                        : "What would you like?"
                    }
                    onChange={(e) => {
                      feedback.edit({ ...draft, description: e.target.value });
                    }}
                  />
                </label>
                <label className="git-field">
                  Remarks (optional)
                  <textarea
                    aria-label="Remarks (optional)"
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
                <div>
                  <label className="feedback-share">
                    <input
                      type="checkbox"
                      data-testid="feedback.screenshot"
                      checked={view?.addScreenshot ?? false}
                      disabled={state.busy}
                      onChange={(e) => void feedback.screenshot(e.target.checked)}
                    />
                    Add a screenshot of Vibe Work
                  </label>
                  {state.capturing && (
                    <p className="key-line" role="status">
                      Taking screenshot…
                    </p>
                  )}
                  {view?.screenshotError && (
                    <p className="key-line" role="status">
                      {view.screenshotError}
                    </p>
                  )}
                  {image && (
                    <div className="feedback-thumbnail">
                      <Button
                        shape="outlined"
                        ref={thumbnail}
                        data-testid="feedback.preview"
                        aria-label="Open screenshot"
                        disabled={state.busy}
                        onClick={() => {
                          feedback.preview(true);
                        }}
                      >
                        <img src={image} alt="Vibe Work screenshot" />
                      </Button>
                    </div>
                  )}
                </div>
                <label className="feedback-share">
                  <input
                    type="checkbox"
                    data-testid="feedback.share"
                    checked={view?.shareSystem ?? false}
                    disabled={state.busy}
                    onChange={(e) => void feedback.share(e.target.checked)}
                  />
                  Add app and system info
                </label>
                {view?.shareSystem && (
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
                )}
                {!view?.available && <p role="status">Sending unavailable in this build</p>}
              </>
            )}
            {state.notice && (
              <p className="key-line" role="status">
                {state.notice}
              </p>
            )}
            {state.error !== null && (
              <p className="key-problem" role="alert">
                {state.error}
              </p>
            )}
          </section>
        )}
        <footer className="modal-actions">
          <Button data-testid="feedback.cancel" disabled={state.busy} onClick={discard}>
            {state.sent ? "Close" : "Cancel"}
          </Button>
          {!state.sent && (
            <Button
              data-testid="feedback.submit"
              variant="primary"
              shape="outlined"
              disabled={
                state.busy || !view?.available || !draft.title.trim() || !draft.description.trim()
              }
              onClick={() => void feedback.send()}
            >
              {state.sending ? "Sending…" : "Send"}
            </Button>
          )}
        </footer>
      </div>
    </div>
  );
}
function ConnectedFeedback({ feedback }: { feedback: Feedback }) {
  useEffect(() => feedback.start(), [feedback]);
  return <FeedbackPanel feedback={feedback} />;
}
/** App-level mount; agent drafts appear through core events, even when another session is selected. */
export function FeedbackDialog() {
  const { core } = useApp();
  return <CoreFeedbackDialog core={core} />;
}
/** The same mount outside a failed shell; no AppProvider is required by the recovery view. */
export function CoreFeedbackDialog({ core }: { core: CoreController | null | undefined }) {
  return !core ? null : <ConnectedFeedback feedback={feedbackFor(core)} />;
}
