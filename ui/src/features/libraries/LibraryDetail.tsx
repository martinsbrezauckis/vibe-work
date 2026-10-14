/**
 * One library: who may use it here (not in chats, all chats, one project), "Add files" (native picker; the
 * files go to Mistral), its documents with their processing state, "Open text" and "Delete" per document, and "Delete
 * library".
 * Depends on: hooks, state/libraries (wording helpers). Called by: LibrariesPage.
 * Invariants: names and summaries are file content shown as text; deleting asks first in the page (a second click),
 * since it deletes on Mistral and cannot be undone.
 */
import { type ReactElement, useState } from "react";

import { Selector } from "../../components/Selector";
import type { LibraryScope } from "../../protocol/core";
import { processing, statusText } from "../../state/libraries";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useLibraries, useLibraryActions } from "./hooks";

/** `off`, `global` or `project:<id>`. */
function scopeKey(scope: LibraryScope): string {
  return scope.kind === "project" ? `project:${scope.projectId}` : scope.kind;
}

/** The scope a picker key stands for. */
export function scopeOf(key: string): LibraryScope {
  if (key.startsWith("project:"))
    return { kind: "project", projectId: key.slice("project:".length) };
  return key === "global" ? { kind: "global" } : { kind: "off" };
}

/** The scope picker. */
export function ScopePicker({
  value,
  label,
  onChange,
  disabled = false,
}: {
  value: LibraryScope;
  label: string;
  onChange: (scope: LibraryScope) => void;
  disabled?: boolean;
}): ReactElement {
  const projects = useLibraries((s) => s.projects);
  return (
    <label className="lib-scope">
      <span>{label}</span>
      <Selector
        label={label}
        value={scopeKey(value)}
        disabled={disabled}
        options={[
          { value: "off", label: "Not in chats" },
          { value: "global", label: "All chats" },
          ...projects.map((project) => ({
            value: `project:${project.id}`,
            label: `Chats in ${project.name}`,
          })),
        ]}
        onChange={(selected) => {
          onChange(scopeOf(selected));
        }}
      />
    </label>
  );
}

function size(bytes: number | null): string {
  if (bytes === null) return "";
  if (bytes < 1024) return `${String(bytes)} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** The selected library, or nothing. */
export function LibraryDetail(): ReactElement | null {
  const blocked = useKeyReadiness() !== null;
  const actions = useLibraryActions();
  const selected = useLibraries((s) => s.selected);
  const library = useLibraries((s) => s.libraries?.find((l) => l.id === s.selected) ?? null);
  const docs = useLibraries((s) => (s.selected === null ? undefined : s.documents[s.selected]));
  const busy = useLibraries((s) => s.busy);
  const [confirm, setConfirm] = useState(false);
  if (selected === null || library === null) return null;
  return (
    <section className="lib-detail" aria-labelledby="lib-detail-title" data-library={library.id}>
      <header className="lib-detail-head">
        <h3 id="lib-detail-title">{library.name}</h3>
        {library.description === null ? null : <p className="cmuted">{library.description}</p>}
      </header>
      <div className="lib-detail-bar">
        <ScopePicker
          value={library.scope}
          label="Used by"
          disabled={blocked}
          onChange={(scope) => void actions?.setScope(library.id, scope)}
        />
        <button
          type="button"
          className="tb-btn primary"
          disabled={blocked || busy === "upload"}
          onClick={() => void actions?.pick(library.id)}
        >
          {busy === "upload" ? "Uploading…" : "Add files"}
        </button>
        <button
          type="button"
          className="tb-btn"
          aria-pressed={confirm}
          disabled={blocked}
          onClick={() => {
            if (confirm) void actions?.deleteLibrary(library.id);
            setConfirm(!confirm);
          }}
        >
          {confirm ? "Delete it and its files on Mistral" : "Delete library"}
        </button>
      </div>
      {docs === undefined ? (
        <p className="cmuted">Reading its documents…</p>
      ) : docs.length === 0 ? (
        <p className="cmuted">
          No documents yet. Add files: PDF, Office, text, CSV, images (up to 100 MB each).
        </p>
      ) : (
        <ul className="lib-docs">
          {docs.map((d) => (
            <li key={d.id} className="lib-doc" data-status={d.status}>
              <div className="lib-doc-main">
                <span className="lib-doc-name">{d.name}</span>
                <span className={`cmuted ${processing(d) ? "lib-processing" : ""}`}>
                  {statusText(d)}
                  {d.pages === null ? "" : ` · ${String(d.pages)} page${d.pages === 1 ? "" : "s"}`}
                  {d.size === null ? "" : ` · ${size(d.size)}`}
                </span>
                {d.summary === null ? null : (
                  <span className="cmuted lib-summary">{d.summary}</span>
                )}
              </div>
              <div className="lib-doc-actions">
                <button
                  type="button"
                  className="tb-btn"
                  disabled={blocked || d.status !== "done"}
                  onClick={() => void actions?.openText(library.id, d.id)}
                >
                  Open text
                </button>
                <button
                  type="button"
                  className="tb-btn"
                  aria-label={`Delete ${d.name}`}
                  disabled={blocked}
                  onClick={() => void actions?.deleteDocument(library.id, d.id)}
                >
                  Delete
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
