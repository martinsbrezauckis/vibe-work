/**
 * The expanded body of one effect step, per `detail.kind` (`web_search`: query,
 * sources, the search answer): `shell` shows the command and
 * the tail of its output; `file_edit` / `file_write` an inline diff; `file_read` the head of what was
 * read; `todo` the list; `subagent` the agent and task; anything else (including kinds newer than
 * this build) the input and output as text. File steps name their file as a File tab reference, and paths in
 * tool output are clickable once the core said they name a file the app may show. Failures show Vibe's error text; Vibe's warnings and
 * approval note are listed.
 * Depends on: model (entries, diff, todos), DiffView, ShowAll. Called by: StepItem.
 * Invariants: everything untrusted is plain text in `<pre>`/text nodes (no Markdown here: tool output
 * is not the model's prose); long output is cut (`ShowAll`).
 */
import type { ReactElement } from "react";

import type { JsonValue } from "../../../protocol/core";
import { term } from "../../../vocabulary";
import { editDiff, writeDiff } from "../model/diff";
import {
  type EffectView,
  type Obj,
  arr,
  entryId,
  inputOf,
  obj,
  outputOf,
  outputText,
  str,
} from "../model/entries";
import { isWebSearch } from "../model/sources";
import { DiffView } from "./DiffView";
import { SearchBody } from "./SearchBody";
import { FileRef } from "../../files/FileRef";
import { ShowAll } from "./ShowAll";

function json(value: JsonValue | undefined): string {
  if (value === undefined || value === null) return "";
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

/** Text output of a shell run: Vibe's `outputText`, else the structured output's streams. */
function shellOutput(entry: Obj): string {
  const text = outputText(entry);
  if (text !== "") return text;
  const out = outputOf(entry);
  return (
    str(out?.["output"]) ?? [str(out?.["stdout"]), str(out?.["stderr"])].filter(Boolean).join("")
  );
}

function KindBody({ entry, view }: { entry: Obj; view: EffectView }): ReactElement | null {
  const id = entryId(entry);
  const input = inputOf(entry);
  const output = outputOf(entry);
  // Web searches (built-in tool or the hosted connector): query + sources instead of raw JSON.
  if (isWebSearch(entry)) return <SearchBody entry={entry} view={view} />;
  switch (view.kind) {
    case "shell": {
      const out = shellOutput(entry);
      return (
        <>
          <pre className="cmd">$ {str(input?.["command"]) ?? view.target}</pre>
          {out !== "" ? <ShowAll id={`${id}:out`} text={out} tail linkify /> : null}
        </>
      );
    }
    case "file_edit": {
      const diff = editDiff(entry);
      return diff === null ? null : <DiffView diff={diff} id={id} />;
    }
    case "file_write": {
      const diff = writeDiff(entry);
      return diff === null ? null : <DiffView diff={diff} id={id} />;
    }
    case "file_read": {
      const content = str(output?.["content"]);
      return content === null ? null : <ShowAll id={`${id}:read`} text={content} />;
    }
    case "todo": {
      const todos = arr(output?.["todos"] ?? input?.["todos"]);
      return (
        <ul className="todo-list">
          {todos.map((raw, i) => {
            const todo = obj(raw);
            const status = str(todo?.["status"]) ?? "pending";
            return (
              <li key={i} data-status={status}>
                <span className="todo-mark" aria-hidden="true" />
                <span className="sr">{status}: </span>
                {str(todo?.["content"]) ?? ""}
              </li>
            );
          })}
        </ul>
      );
    }
    case "subagent": {
      const detail = obj(entry["detail"]);
      return (
        <dl className="kv">
          <dt>Agent</dt>
          <dd>
            {
              term("accessModes", str(detail?.["agentName"]) ?? str(input?.["agent"]) ?? "subagent")
                .name
            }
          </dd>
          <dt>Task</dt>
          <dd>{str(input?.["task"]) ?? ""}</dd>
        </dl>
      );
    }
    default: {
      const inText = json(input ?? undefined);
      const outText = outputText(entry) || json(output ?? undefined);
      return (
        <>
          {inText !== "" ? <ShowAll id={`${id}:in`} text={inText} className="out in" /> : null}
          {outText !== "" ? <ShowAll id={`${id}:out`} text={outText} linkify /> : null}
        </>
      );
    }
  }
}

/** The file a `file_read` / `file_write` / `file_edit` step touched, as Vibe wrote its path. */
function filePathOf(entry: Obj, view: EffectView): string | null {
  if (view.kind !== "file_read" && view.kind !== "file_write" && view.kind !== "file_edit")
    return null;
  return (
    str(inputOf(entry)?.["filePath"]) ??
    str(outputOf(entry)?.["filePath"]) ??
    str(outputOf(entry)?.["file"])
  );
}

/** The body of one effect step. */
export function StepBody({ entry, view }: { entry: Obj; view: EffectView }): ReactElement {
  const file = filePathOf(entry, view);
  return (
    <div className="step-b">
      {file !== null && file !== "" ? (
        <p className="step-file">
          File:{" "}
          <FileRef path={file} line={null}>
            {file}
          </FileRef>
        </p>
      ) : null}
      {view.error !== null ? (
        <p className="key-problem" role="alert">
          {view.error}
        </p>
      ) : null}
      <KindBody entry={entry} view={view} />
      {view.warnings.length > 0 ? (
        <ul className="server-note" role="status">
          {view.warnings.map((w, i) => (
            <li key={i}>{w}</li>
          ))}
        </ul>
      ) : null}
      {view.approvalNote !== null && view.approvalNote !== "" ? (
        <p className="step-note">{view.approvalNote}</p>
      ) : null}
    </div>
  );
}
