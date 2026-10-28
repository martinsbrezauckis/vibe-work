/** Files panel presentation, composed from the app's selectors, fields, row controls and notices. */
import { useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Button, LinkButton } from "../../components/Controls";
import { EmptyState } from "../../components/EmptyState";
import { Icon } from "../../components/Icon";
import { RowMenu, useRowMenu } from "../../components/RowMenu";
import { PathText } from "../../components/PathText";
import { Selector, type SelectorOption } from "../../components/Selector";
import { useStore } from "../../state/createStore";
import { formatKeys } from "../../state/shortcuts";
import type { ExplorerVersion } from "../../protocol/core";
import { FileTree } from "./FileTree";
import { History, n } from "./History";
import { SearchResults } from "./SearchResults";
import { shownResults, type Explorer } from "./store";
import { type ExplorerMenuDeps, type ExplorerTarget, explorerMenu } from "./explorerMenus";

export interface ExplorerViewProps {
  explorer: Explorer;
  places: SelectorOption<string>[];
  onPlace: (folder: string) => void;
  onOpen: (path: string, line?: number) => void;
  onReview: () => void;
  onChoose: () => void;
  onInit: () => void;
  session: boolean;
  legacy: ReactNode;
  workspaceActions: ReactNode;
  /** What the right-click menus act with; without it (no folder owner yet) rows have no menu. */
  menuDeps?: ExplorerMenuDeps | null;
  /** A file inside a commit: compare it with that commit's version (the default: open it as it is now). */
  onCompare?: (path: string, version: ExplorerVersion) => void;
}
export function ExplorerView(props: ExplorerViewProps) {
  const { explorer } = props,
    state = useStore(explorer.store, (s) => s),
    search = useRef<HTMLInputElement>(null);
  const rowMenu = useRowMenu<ExplorerTarget>();
  const menu = props.menuDeps ? rowMenu : null;
  const built = menu?.at && props.menuDeps ? explorerMenu(props.menuDeps, menu.at.target) : null;
  const changed = (state.status?.changedFiles ?? 0) + (state.status?.untrackedFiles ?? 0);
  const count = state.paths.length;
  function first() {
    const shown = shownResults(state),
      path = shown.names[0],
      hit = shown.hits[0],
      version = state.searchVersions[0];
    if (path) props.onOpen(path);
    else if (hit) props.onOpen(hit.path, hit.line);
    else if (version) void explorer.openFound(version);
  }
  if (!state.folder)
    return (
      <EmptyState glyph="folder" small>
        <p>Choose a folder to see its files.</p>
        <Button data-testid="explorer.choose-folder" shape="outlined" onClick={props.onChoose}>
          Choose folder…
        </Button>
        {props.places.length > 0 && (
          <Selector
            testId="explorer.initial-place"
            label="Places"
            value=""
            options={[{ value: "", label: "Choose a place" }, ...props.places]}
            onChange={props.onPlace}
          />
        )}
      </EmptyState>
    );
  return (
    <section className="fx-panel" aria-label="Files explorer">
      <div className="fx-header">
        <div className="fx-place">
          <Icon name="folder" size={16} />
          <PathText path={state.folder} />
          <Selector
            testId="explorer.place"
            compact
            label="Places"
            value={state.folder}
            options={props.places}
            onChange={props.onPlace}
          />
        </div>
        <p className="ui-help">
          {state.status ? (
            <>
              {state.commitCount != null && (
                <>
                  {n(state.commitCount)} {state.commitCount === 1 ? "commit" : "commits"} ·{" "}
                </>
              )}
              {state.latest && (
                <>
                  last {new Date(state.latest.time * 1000).toLocaleDateString()} by{" "}
                  {state.latest.who} ·{" "}
                </>
              )}
              {state.status.branch ?? "No branch selected"} · {n(changed)}{" "}
              {changed === 1 ? "file" : "files"} changed since the last commit ·{" "}
              {state.status.upstream
                ? state.status.ahead > 0
                  ? `${n(state.status.ahead)} ${state.status.ahead === 1 ? "commit" : "commits"} not sent yet`
                  : "up to date with the shared history"
                : "only on this computer"}
            </>
          ) : (
            `${n(count)}${state.indexLimited ? "+" : ""} files · no version history`
          )}
        </p>
        <div className="fx-search set-search">
          <Icon name="search" size={14} />
          <input
            ref={search}
            data-testid="explorer.search"
            aria-label="Search files, text and commits"
            placeholder="Search files, text and commits"
            title={`Search files (${formatKeys("Mod+Shift+F")})`}
            value={state.query}
            onChange={(e) => {
              explorer.query(e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                explorer.clearSearch();
              }
              if (e.key === "Enter") {
                e.preventDefault();
                first();
              }
              if (e.key === "ArrowDown") {
                e.preventDefault();
                document.querySelector<HTMLButtonElement>(".fx-results button")?.focus();
              }
            }}
          />
        </div>
        <div className="fx-tools">
          <Selector
            testId="explorer.view"
            compact
            label="View"
            value={state.view}
            options={[
              { value: "files", label: "Files" },
              { value: "history", label: "History" },
            ]}
            onChange={(value) => {
              explorer.setView(value);
            }}
          />
          {state.view === "files" && !state.query && state.status !== null && (
            <label className="ui-caption">
              <input
                data-testid="explorer.changed-only"
                type="checkbox"
                checked={state.changedOnly}
                onChange={(e) => {
                  explorer.changedOnly(e.target.checked);
                }}
              />{" "}
              Changed only
            </label>
          )}
          <Button
            data-testid="explorer.refresh"
            onClick={() => {
              explorer.refresh();
            }}
          >
            Refresh
          </Button>
          <details>
            <summary data-testid="explorer.more" aria-label="More file options">
              More
            </summary>
            <label className="ui-caption">
              <input
                data-testid="explorer.show-ignored"
                type="checkbox"
                checked={state.showIgnored}
                onChange={(e) => {
                  explorer.showIgnored(e.target.checked);
                }}
              />{" "}
              Show ignored
            </label>
          </details>
        </div>
        {state.changedOnly && count > 5000 && (
          <p className="ui-help fx-info" data-testid="explorer.large-notice">
            <Icon name="info" size={14} />
            <span>
              Large project ({n(count)} files): showing changed files. Turn off Changed only to
              browse everything.
            </span>
          </p>
        )}
        {state.scope !== null && (
          <p className="ui-help fx-info" data-testid="explorer.scope">
            <Icon name="folder" size={14} />
            <span>Searching in {state.scope}/ only.</span>
            <LinkButton
              data-testid="explorer.scope-clear"
              onClick={() => {
                explorer.searchIn(null);
              }}
            >
              Search everywhere
            </LinkButton>
          </p>
        )}
        {state.indexing && (
          <p className="key-line" role="status">
            Indexing file names…
          </p>
        )}
        {state.error && (
          <p className="key-problem" role="alert">
            {state.error}
          </p>
        )}
      </div>
      {state.query.trim() ? (
        <SearchResults state={state} explorer={explorer} onOpen={props.onOpen} menu={menu} />
      ) : state.view === "history" ? (
        <History
          state={state}
          explorer={explorer}
          session={props.session}
          onOpen={props.onOpen}
          onReview={props.onReview}
          onInit={props.onInit}
          places={props.legacy}
          workspaceActions={props.workspaceActions}
          menu={menu}
          onCompare={
            props.onCompare ??
            ((path) => {
              props.onOpen(path);
            })
          }
        />
      ) : (
        <>
          <FileTree state={state} explorer={explorer} onOpen={props.onOpen} menu={menu} />
          <div className="fx-footer ui-caption" role="status">
            {n(count)}
            {state.indexLimited ? "+" : ""} files · {n(changed)} changed
            {!state.showIgnored && (state.folders[""]?.ignored.length ?? 0) > 0
              ? ` · ${state.folders[""]?.ignored.join(", ") ?? ""} hidden (ignored)`
              : ""}
          </div>
        </>
      )}
      {menu?.at &&
        built &&
        createPortal(
          <RowMenu
            testId="explorer.menu"
            at={menu.at}
            label={built.label}
            heading={built.heading}
            items={built.items}
            onClose={menu.close}
          />,
          document.body,
        )}
    </section>
  );
}
