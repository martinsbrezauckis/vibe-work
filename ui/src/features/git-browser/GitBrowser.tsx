/** Virtualized project git explorer. Depends on virtua and browser model; called by the Git panel adapter. */
import { Button } from "../../components/Controls";
import { useMemo, useRef, useState, type KeyboardEvent, type ReactElement } from "react";
import { VList, type VListHandle } from "virtua";
import { NodeActions } from "./NodeActions";
import { FileActions } from "./FileActions";
import {
  stateWords,
  fileWords,
  type BrowserAction,
  type BrowserRepo,
  type BrowserTarget,
} from "./model";
import { treeRows, type BrowserRow } from "./tree";

export type GitBrowserProps = {
  repos: BrowserRepo[];
  inHistory?: boolean;
  selectedSessionId?: string | null;
  onAction: (target: BrowserTarget, action: BrowserAction) => void;
  onFile: (target: BrowserTarget, path: string) => void;
  onPoint?: (target: BrowserTarget, path: string) => void;
  loading?: boolean;
  error?: string | null;
  note?: string | null;
  onRetry?: () => void;
};

export function GitBrowser({
  repos,
  inHistory = false,
  selectedSessionId = null,
  onAction,
  onFile,
  onPoint,
  loading = false,
  error = null,
  note = null,
  onRetry,
}: GitBrowserProps): ReactElement {
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [selected, setSelected] = useState<string | null>(null);
  const list = useRef<VListHandle>(null);
  const elements = useRef(new Map<string, HTMLDivElement>());
  const pending = useRef<string | null>(null);
  const rows = useMemo(() => treeRows(repos, collapsed), [repos, collapsed]);
  const active = rows.some((row) => row.id === selected) ? selected : (rows[0]?.id ?? null);
  function toggle(row: BrowserRow) {
    setCollapsed((previous) => {
      const next = new Set(previous);
      if (next.has(row.id)) next.delete(row.id);
      else next.add(row.id);
      return next;
    });
  }
  function focus(row: BrowserRow | undefined) {
    if (row === undefined) return;
    setSelected(row.id);
    pending.current = row.id;
    list.current?.scrollToIndex(rows.indexOf(row), { align: "nearest" });
    const element = elements.current.get(row.id);
    if (element) {
      element.focus();
      pending.current = null;
    }
  }
  function keyboard(event: KeyboardEvent<HTMLDivElement>, row: BrowserRow) {
    if (event.target !== event.currentTarget) return;
    const index = rows.indexOf(row);
    switch (event.key) {
      case "ArrowDown":
        focus(rows[Math.min(index + 1, rows.length - 1)]);
        break;
      case "ArrowUp":
        focus(rows[Math.max(index - 1, 0)]);
        break;
      case "Home":
        focus(rows[0]);
        break;
      case "End":
        focus(rows.at(-1));
        break;
      case "ArrowRight":
        if (row.expandable && collapsed.has(row.id)) toggle(row);
        else if (rows[index + 1]?.parent === row.id) focus(rows[index + 1]);
        break;
      case "ArrowLeft":
        if (row.expandable && !collapsed.has(row.id)) toggle(row);
        else focus(rows.find((candidate) => candidate.id === row.parent));
        break;
      case "Enter":
        if (row.file) onFile(row.target, row.file.path);
        else if (row.expandable) toggle(row);
        break;
      default:
        return;
    }
    event.preventDefault();
  }
  return (
    <section className="git-browser" aria-label="Project git browser" aria-busy={loading}>
      {loading && (
        <p className="gb-message" role="status">
          Reading repositories…
        </p>
      )}
      {error !== null && (
        <div className="key-problem" role="alert">
          {error}
          {onRetry && (
            <Button
              data-testid="git-browser.retry"
              appearance="compact"
              type="button"
              onClick={onRetry}
            >
              Retry
            </Button>
          )}
        </div>
      )}
      {rows.length === 0 && !loading && error === null && (
        <p className="gb-message">No repositories found in this project.</p>
      )}
      {note !== null && (
        <p className="server-note" role="status">
          {note}
        </p>
      )}
      {rows.length > 0 && (
        <VList
          ref={list}
          data={rows}
          className="gb-tree"
          role="tree"
          aria-label="Repositories and copies"
        >
          {(row: BrowserRow) => (
            <div
              key={row.id}
              data-testid="git-browser.row"
              data-item-id={row.id}
              role="treeitem"
              className={`gb-row gb-level-${String(row.level)}`}
              data-node-id={row.id}
              data-node-kind={row.kind}
              aria-level={row.level}
              aria-posinset={row.position}
              aria-setsize={row.size}
              aria-expanded={row.expandable ? !collapsed.has(row.id) : undefined}
              aria-selected={active === row.id}
              tabIndex={active === row.id ? 0 : -1}
              ref={(element) => {
                if (element) {
                  elements.current.set(row.id, element);
                  if (pending.current === row.id) {
                    element.focus();
                    pending.current = null;
                  }
                } else elements.current.delete(row.id);
              }}
              onFocus={(event) => {
                if (event.target === event.currentTarget) setSelected(row.id);
              }}
              onKeyDown={(event) => {
                keyboard(event, row);
              }}
              onClick={(event) => {
                if ((event.target as HTMLElement).closest("button, summary, details")) return;
                focus(row);
                if (row.file) onFile(row.target, row.file.path);
                else if (row.expandable) toggle(row);
              }}
            >
              <div
                className="gb-heading"
                title={row.file ? `${row.target.path}/${row.file.path}` : row.target.path}
              >
                <span className="gb-chevron" aria-hidden="true">
                  {row.expandable ? (collapsed.has(row.id) ? "›" : "⌄") : ""}
                </span>
                <span className="gb-name">{row.name}</span>
                {row.file && <span className="gb-file-state">{fileWords(row.file.status)}</span>}
              </div>
              {/* A copy outside the project offers only its chat; pointing at its files is always refused. */}
              {row.file && !row.target.restricted && (
                <FileActions
                  target={row.target}
                  path={row.file.path}
                  onPoint={
                    onPoint === undefined
                      ? undefined
                      : () => {
                          if (row.file) onPoint(row.target, row.file.path);
                        }
                  }
                />
              )}
              {(row.kind === "repository" || row.kind === "copy") && (
                <>
                  <p className="gb-state">
                    {row.target.branch ??
                      row.target.observed?.branch ??
                      (row.target.status?.state?.head.kind === "branch"
                        ? row.target.status.state.head.name
                        : "No branch selected")}{" "}
                    · {stateWords(row.target)}
                  </p>
                  <NodeActions
                    target={row.target}
                    inHistory={inHistory}
                    selectedSessionId={selectedSessionId}
                    onAction={onAction}
                  />
                </>
              )}
            </div>
          )}
        </VList>
      )}
    </section>
  );
}
