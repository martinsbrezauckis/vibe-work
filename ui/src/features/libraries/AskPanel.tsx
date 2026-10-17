/**
 * "Ask your libraries": one question across every library that is used in chats, the written answer and
 * its numbered sources; a source opens the document's text (`TextViewer`).
 * Depends on: hooks. Called by: LibrariesPage.
 * Invariants: the answer, titles and passages are file content: rendered as plain text, never as HTML or Markdown
 * links (a document cannot add a link or a script to the page).
 */
import type { ReactElement, SyntheticEvent } from "react";

import { useKeyReadiness } from "../shell/KeyNotice";
import { useLibraries, useLibraryActions } from "./hooks";

function Sources(): ReactElement | null {
  const actions = useLibraryActions();
  const answer = useLibraries((s) => s.answer);
  if (answer === null || answer.sources.length === 0) return null;
  return (
    <ol className="lib-sources" aria-label="Sources">
      {answer.sources.map((s) => (
        <li data-item-id={s.n} key={s.n} data-source={s.n}>
          <button
            data-testid="libraries.ask-panel-open-text"
            type="button"
            className="lib-source"
            disabled={s.libraryId === null}
            title={
              s.libraryId === null ? "This document is no longer in a library" : "Open its text"
            }
            onClick={() => {
              if (s.libraryId !== null) void actions?.openText(s.libraryId, s.documentId, s.n);
            }}
          >
            <span className="mono">[{s.n}]</span> {s.title}
          </button>
          <p className="cmuted lib-snippet">{s.snippet}</p>
        </li>
      ))}
    </ol>
  );
}

/** The panel. */
export function AskPanel(): ReactElement {
  const blocked = useKeyReadiness() !== null;
  const actions = useLibraryActions();
  const question = useLibraries((s) => s.question);
  const answer = useLibraries((s) => s.answer);
  const busy = useLibraries((s) => s.busy);
  const usable = useLibraries(
    (s) => (s.libraries ?? []).filter((l) => l.scope.kind !== "off").length,
  );
  const submit = (e: SyntheticEvent) => {
    e.preventDefault();
    if (!blocked && question.trim() !== "") void actions?.ask(question.trim());
  };
  return (
    <section className="lib-ask" aria-labelledby="lib-ask-title">
      <h3 id="lib-ask-title">Ask your libraries</h3>
      <form className="lib-ask-row" onSubmit={submit}>
        <input
          data-testid="libraries.ask-panel-set-question"
          value={question}
          aria-label="Question"
          placeholder={
            usable === 0 ? "Switch a library on for chats first" : "What do my documents say about…"
          }
          disabled={usable === 0}
          onChange={(e) => actions?.setQuestion(e.target.value)}
        />
        <button
          data-testid="libraries.ask-panel-tb-btn"
          type="submit"
          className="tb-btn primary"
          disabled={blocked || usable === 0 || busy === "ask" || question.trim() === ""}
        >
          {busy === "ask" ? "Searching…" : "Ask"}
        </button>
      </form>
      {answer === null ? null : (
        <div className="lib-answer" data-testid="library-answer">
          <p className="lib-answer-text">{answer.answer}</p>
          <Sources />
          <p className="cmuted">
            {answer.queries.length} search{answer.queries.length === 1 ? "" : "es"},{" "}
            {answer.tokens.toLocaleString()} tokens. The agent in your chats searches the same
            libraries.
          </p>
        </div>
      )}
    </section>
  );
}
