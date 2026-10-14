/**
 * The pinned todo strip above the transcript (like the CLI's `/todo`), derived from the
 * newest `todo` effect (`model/todos.ts`). Collapsed it shows progress and the item in progress;
 * opened, the whole list.
 * Depends on: model/todos, the transcript store. Called by: Transcript.
 */
import type { ReactElement } from "react";

import { useApp } from "../shell/AppContext";
import { useTranscriptUi } from "./hooks";
import { type Todo, todoProgress } from "./model/todos";

const KEY = "todo-strip";

/** Renders the strip for `todos`. */
export function TodoStrip({ todos }: { todos: readonly Todo[] }): ReactElement {
  const { transcript } = useApp();
  const open = useTranscriptUi((s) => s.open[KEY] === true);
  const { done, total } = todoProgress(todos);
  const current =
    todos.find((t) => t.status === "in_progress") ?? todos.find((t) => t.status === "pending");
  const label = open ? "Hide todo list" : "Show todo list";
  return (
    <div className="todo-strip">
      <button
        type="button"
        className="todo-h"
        aria-expanded={open}
        aria-label={label}
        title={label}
        onClick={() => {
          transcript.toggle(KEY);
        }}
      >
        <span className="caret" aria-hidden="true" />
        <b>Todo</b>
        <span className="todo-count">
          {String(done)}/{String(total)}
        </span>
        {current !== undefined ? <span className="todo-now">{current.content}</span> : null}
      </button>
      {open ? (
        <ul className="todo-list">
          {todos.map((todo) => (
            <li key={todo.id} data-status={todo.status}>
              <span className="todo-mark" aria-hidden="true" />
              <span className="sr">{todo.status}: </span>
              {todo.content}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
