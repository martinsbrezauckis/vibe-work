/**
 * The Libraries page: documents uploaded once to the user's Mistral account and asked across, by the
 * user here and by the agent in chats (only libraries switched on for all chats or the chat's project). Shown in the
 * content area from the rail. Says plainly what leaves the computer and the request limits (the core's sentences).
 * Depends on: hooks, AskPanel, LibraryDetail, TextViewer. Called by: Content.
 * Invariants: components render, the libraries controller decides; file content is text only.
 */
import { type ReactElement, type SyntheticEvent, useEffect, useRef, useState } from "react";

import type { LibraryScope } from "../../protocol/core";
import { scopeText } from "../../state/libraries";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { AskPanel } from "./AskPanel";
import { LibraryDetail, ScopePicker } from "./LibraryDetail";
import { TextViewer } from "./TextViewer";
import { useLibraries, useLibraryActions } from "./hooks";

function NewLibrary(): ReactElement {
  const blocked = useKeyReadiness() !== null;
  const actions = useLibraryActions();
  const busy = useLibraries((s) => s.busy);
  const [name, setName] = useState("");
  const [scope, setScope] = useState<LibraryScope>({ kind: "global" });
  const submit = (e: SyntheticEvent) => {
    e.preventDefault();
    if (blocked || name.trim() === "") return;
    void actions?.create(name.trim(), scope, null).then((ok) => {
      if (ok) setName("");
    });
  };
  return (
    <form className="lib-new" onSubmit={submit} aria-label="New library">
      <input
        data-testid="libraries.libraries-page-set-name"
        value={name}
        placeholder="New library name"
        aria-label="New library name"
        maxLength={100}
        onChange={(e) => {
          setName(e.target.value);
        }}
      />
      <ScopePicker
        testId="libraries.create-scope"
        value={scope}
        label="Used by"
        onChange={setScope}
        disabled={blocked}
      />
      <button
        data-testid="libraries.libraries-page-tb-btn"
        type="submit"
        className="tb-btn"
        disabled={blocked || busy === "create" || name.trim() === ""}
      >
        Make library
      </button>
    </form>
  );
}

function List(): ReactElement {
  const actions = useLibraryActions();
  const libraries = useLibraries((s) => s.libraries);
  const projects = useLibraries((s) => s.projects);
  const selected = useLibraries((s) => s.selected);
  const loading = useLibraries((s) => s.loading);
  if (libraries === null)
    return <p className="cmuted">{loading ? "Reading your libraries…" : ""}</p>;
  if (libraries.length === 0)
    return <p className="cmuted">No libraries yet. Make one, then add files to it.</p>;
  return (
    <ul className="lib-list" aria-label="Libraries">
      {libraries.map((l) => (
        <li data-item-id={l.id} key={l.id}>
          <button
            data-testid="libraries.libraries-page-select"
            type="button"
            className="lib-item"
            aria-current={selected === l.id ? "true" : undefined}
            onClick={() => void actions?.select(l.id)}
          >
            <span className="lib-item-name">{l.name}</span>
            <span className="cmuted">
              {l.documents} document{l.documents === 1 ? "" : "s"} · {scopeText(l.scope, projects)}
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}

/** The page. */
export function LibrariesPage(): ReactElement {
  const blocked = useKeyReadiness() !== null;
  const actions = useLibraryActions();
  const wasBlocked = useRef(blocked);
  useEffect(() => {
    if (wasBlocked.current && !blocked) void actions?.open();
    wasBlocked.current = blocked;
  }, [actions, blocked]);
  const costs = useLibraries((s) => s.costs);
  const error = useLibraries((s) => s.error);
  const note = useLibraries((s) => s.note);
  return (
    <div className="cpage lib-page" data-testid="libraries-page">
      <div className="cmain">
        <div className="chead">
          <h2>Libraries</h2>
          <p className="cmuted">
            Upload documents once and ask across them, here or in any chat that may use the library.
            Answers cite their sources.
          </p>
          {costs === null ? null : (
            <ul className="lib-costs cmuted" aria-label="What this sends and its limits">
              <li>{costs.leaves}</li>
              <li>{costs.limits}</li>
            </ul>
          )}
        </div>
        {blocked && (
          <KeyNotice message="Libraries use your Mistral key, which is kept in the vault." />
        )}
        {blocked || error === null ? null : (
          <p className="server-note" role="status">
            <span className="px warn" aria-hidden="true" />
            <span>{error}</span>
          </p>
        )}
        {blocked || note === null ? null : (
          <p className="cnote" role="status">
            {note}
          </p>
        )}
        <AskPanel />
        <div className="lib-columns">
          <div className="lib-side">
            <NewLibrary />
            <List />
          </div>
          <LibraryDetail />
        </div>
      </div>
      <TextViewer />
    </div>
  );
}
