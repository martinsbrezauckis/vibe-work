/** File names, source lines and commits from the one Files search. Uses shared paths and controls; called by ExplorerView. */
import { LinkButton } from "../../components/Controls";
import { PathText } from "../../components/PathText";
import { RowMenuButton, type RowMenuState } from "../../components/RowMenu";
import type { ExplorerTarget } from "./explorerMenus";
import { n, VersionRow } from "./History";
import { NAME_LIMIT, shownResults, type Explorer, type ExplorerState } from "./store";
export function SearchResults({
  state,
  explorer,
  onOpen,
  menu = null,
}: {
  state: ExplorerState;
  explorer: Explorer;
  onOpen: (path: string, line?: number) => void;
  /** The tab's one menu (right click, menu key, ⋯); null: no menu. */
  menu?: RowMenuState<ExplorerTarget> | null;
}) {
  const { names, hits } = shownResults(state);
  const count = names.length + hits.length + state.searchVersions.length;
  const more = (target: ExplorerTarget, id: string) =>
    menu && (
      <RowMenuButton
        testId="explorer.result-more"
        itemId={id}
        label={`Actions for ${id}`}
        expanded={menu.at !== null && JSON.stringify(menu.at.target) === JSON.stringify(target)}
        onOpen={(button) => {
          menu.openBelow(button, target);
        }}
      />
    );
  return (
    <div className="fx-scroll fx-results">
      <p className="key-line" role="status" aria-live="polite">
        {n(count)} results{state.searching ? " · Searching…" : ""}
      </p>
      {names.length > 0 && <h3 className="ui-caption">Files · {n(names.length)}</h3>}
      {names.map((path) => (
        <div
          className="fx-result"
          key={path}
          {...menu?.triggers({ kind: "file", path, place: "search" })}
        >
          <LinkButton
            data-testid="explorer.name-result"
            data-item-id={path}
            onClick={() => {
              onOpen(path);
            }}
          >
            <PathText path={path} />
          </LinkButton>
          {more({ kind: "file", path, place: "search" }, path)}
        </div>
      ))}
      {names.length === NAME_LIMIT && (
        <p className="ui-help">First 200 files. Refine the search to see others.</p>
      )}
      {hits.length > 0 && <h3 className="ui-caption">Text in files · {n(hits.length)}</h3>}
      {hits.map((hit) => (
        <div
          className="fx-result"
          key={`${hit.path}:${String(hit.line)}`}
          {...menu?.triggers({ kind: "hit", path: hit.path, line: hit.line, text: hit.text })}
        >
          <LinkButton
            data-testid="explorer.text-result"
            data-item-id={`${hit.path}:${String(hit.line)}`}
            onClick={() => {
              onOpen(hit.path, hit.line);
            }}
          >
            <PathText path={`${hit.path}:${String(hit.line)}`} />
          </LinkButton>
          {more(
            { kind: "hit", path: hit.path, line: hit.line, text: hit.text },
            `${hit.path}:${String(hit.line)}`,
          )}
          <p className="ui-help" title={hit.text}>
            {hit.text}
          </p>
        </div>
      ))}
      {state.searchLimited && (
        <p className="key-notice">
          {state.hits.length >= 500
            ? "First 500 matches. Refine the search to see others."
            : "Search reached a reading limit. Refine the search to see others."}
        </p>
      )}
      {state.searchVersions.length > 0 && (
        <h3 className="ui-caption">Commits · {n(state.searchVersions.length)}</h3>
      )}
      {state.searchVersions.map((version) => (
        <VersionRow
          key={version.id}
          version={version}
          onOpen={() => void explorer.openFound(version)}
          menu={menu}
        />
      ))}
      {state.searchVersions.length === 50 && (
        <p className="ui-help">First 50 matching commits. Refine the search to see others.</p>
      )}
      {count === 0 && !state.searching && (
        <p className="key-line">Nothing matches “{state.query}”.</p>
      )}
    </div>
  );
}
