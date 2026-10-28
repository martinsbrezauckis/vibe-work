/**
 * The global search (Ctrl K, the title bar's field). It uses fuzzy filtering, arrows/Enter/Esc and combobox semantics. The palette groups results and keeps keyboard navigation consistent:
 * - plain-text results in fixed groups: Sessions, Messages, Files, Settings; `/` searches Actions only.
 *   The palette adds Messages and Files from the core's `search` command (`features/search/`).
 *   Each row has a type icon, the title with the matched words marked,
 *   and a path to the part that holds the match (`repo › Add rate limiting › Agent reply, turn 4`,
 *   `repo › config.toml:8`, `Settings › Voice`, `Actions › Project`);
 * - a preview of the highlighted result beside the list (hidden in narrow windows);
 * - type filters with counts under the field; Tab / Shift Tab move between them; "Show all" opens one type;
 * - each result opens as what it is: a session where it was left, a message in its conversation at the match with
 *   the find bar open, a file in the File tab at the line, a setting in the Settings page at that setting (lit for a
 *   moment), an action runs. While the message index fills, a line says so and the results grow.
 * Adapted from ShellX.
 * Depends on: AppContext, hooks, state/commands, state/shortcuts, hooks/useModalFocus, settings/catalog,
 * components/Icon, components/Highlight. Called by: Shell.
 * Invariants: opens fresh every time (selection reset; the query starts with the words a scoped search carried
 * over, else empty); Enter runs the highlighted entry and closes; Escape and a click outside close; focus returns
 * to where it was (the modal-focus hook). With an empty query the list shows every action, then every session, as
 * before (nothing hides behind a filter). Selection follows the result identity while answers arrive; surviving
 * rows update in place and new hits append within their group.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useEffect, useMemo, useRef, useState } from "react";

import { Highlight } from "../../components/Highlight";
import { Icon } from "../../components/Icon";
import { Selector } from "../../components/Selector";
import { useModalFocus } from "../../hooks/useModalFocus";
import { SEARCH_AUTOLOAD_PAGES } from "../../state/coreStore";
import { indexNote } from "../search/coreRows";
import { PaletteEmpty } from "../search/PaletteEmpty";
import { PalettePreview } from "../search/PalettePreview";
import { GROUPS, type Kind, type Result } from "../search/paletteTypes";
import { TEXT_GROUPS, type Filter, paletteQuery, paletteRows } from "../search/paletteModel";
import { publishResults } from "../search/searchRoutes";
import { useCoreSearch } from "../search/useCoreSearch";
import { usePaletteResults } from "../search/usePaletteResults";
import { usePaletteSelection } from "../search/usePaletteSelection";
import { useApp } from "./AppContext";
import { useCoreState, useShellState } from "./hooks";

const LISTBOX = "vw-palette-list";

const FILTERS: readonly Filter[] = ["all", ...TEXT_GROUPS];

/** The palette (renders nothing while closed). */
export function CommandPalette() {
  const open = useShellState((s) => s.overlay === "palette");
  if (!open) return null;
  return <PaletteDialog />;
}

function PaletteDialog(): ReactElement {
  const app = useApp();
  const { shell, core } = app;
  const nextCursor = useCoreState((s) => s.index.nextCursor);
  const loading = useCoreState((s) => s.index.loading);
  const carried = useShellState((s) => s.paletteQuery);
  const loadedPages = useRef(0);
  const dialog = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const [query, setQuery] = useState(carried);
  const filesOnly = useShellState((s) => s.paletteFiles);
  const [filter, setFilter] = useState<Filter>(filesOnly ? "file" : "all");
  const close = () => {
    shell.actions.setOverlay(null);
  };
  useModalFocus(true, dialog, close);
  const parsed = paletteQuery(query);
  const { mode, text } =
    filesOnly && parsed.mode === "empty" ? { mode: "text" as const, text: "" } : parsed;
  const searching = mode !== "empty";
  const found = useCoreSearch(mode === "text" ? text : "");
  const byKind = usePaletteResults(query, found);
  const rows = useMemo(() => paletteRows(byKind, mode, filter), [byKind, mode, filter]);
  const { active, selected, select } = usePaletteSelection(rows);

  // Searching covers every page: while a query is typed and Vibe has older pages, load them (capped).
  useEffect(() => {
    if (mode !== "text" || core === null || nextCursor === null || loading) return;
    if (loadedPages.current >= SEARCH_AUTOLOAD_PAGES) return;
    loadedPages.current += 1;
    void core.loadMore();
  }, [core, mode, nextCursor, loading]);

  // The `ui/search` route reads the rows as shown and runs one like Enter.
  useEffect(() => {
    publishResults(
      app,
      query,
      rows.map((r) => r.result),
    );
  });

  useEffect(() => {
    const option = list.current?.querySelector(`#${optionId(active)}`);
    // jsdom has no scrollIntoView; real windows do.
    if (option !== null && option !== undefined && typeof option.scrollIntoView === "function")
      option.scrollIntoView({ block: "nearest" });
  }, [active, selected?.id]);

  const run = (item: Result | undefined) => {
    if (item === undefined) return;
    close();
    item.run();
  };
  const cycle = (step: 1 | -1) => {
    const at = FILTERS.indexOf(filter);
    const next = FILTERS[(at + step + FILTERS.length) % FILTERS.length] ?? "all";
    setFilter(next);
  };
  const count = (kind: Kind) =>
    kind === "message"
      ? Math.max(found.messageTotal, byKind.message.length)
      : kind === "file"
        ? Math.max(found.fileTotal, byKind.file.length)
        : byKind[kind].length;
  const note =
    mode === "text"
      ? (found.error ??
        indexNote(found) ??
        (found.loading ? "Searching messages and files…" : null))
      : null;

  return (
    <>
      <div data-testid="shell.command-palette-scrim" className="scrim" onClick={close} />
      <div
        data-testid="shell.command-palette-palette"
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        ref={dialog}
        tabIndex={-1}
      >
        <div className="pal-in">
          <Icon name="search" />
          <input
            data-testid="shell.command-palette-set-query"
            data-dialog-initial-focus="true"
            data-dialog-manages-tab={mode === "text" ? "true" : undefined}
            // Focus during commit (before paint), so keys typed right after Ctrl K are never lost.
            autoFocus
            role="combobox"
            aria-label="Search commands"
            aria-description="Search sessions, messages, files and settings. Start with / for actions."
            aria-expanded="true"
            aria-controls={LISTBOX}
            aria-activedescendant={rows.length > 0 ? optionId(active) : undefined}
            aria-autocomplete="list"
            placeholder="Search sessions, messages, files and settings; / for actions"
            autoComplete="off"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                select(Math.min(active + 1, rows.length - 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                select(Math.max(active - 1, 0));
              } else if (e.key === "Enter") {
                e.preventDefault();
                run(rows[active]?.result);
              } else if (e.key === "Tab" && mode === "text") {
                // This input owns Tab in text mode: cycle the result filter and keep typing focus.
                e.preventDefault();
                cycle(e.shiftKey ? -1 : 1);
              }
            }}
          />
        </div>
        {mode === "text" && (
          <div className="pal-f">
            <Selector
              testId="shell.command-palette-selector"
              label="Show results"
              value={filter}
              options={FILTERS.map((f) => ({
                value: f,
                label:
                  f === "all"
                    ? "All"
                    : `${GROUPS.find((g) => g.kind === f)?.label ?? f} ${String(count(f))}`,
              }))}
              onChange={setFilter}
              compact
            />
          </div>
        )}
        <div className="pal-b">
          <ul
            data-testid="shell.command-palette-plist"
            className="plist"
            id={LISTBOX}
            role="listbox"
            aria-label="Commands"
            ref={list}
          >
            <PaletteEmpty
              query={text}
              mode={mode}
              filter={filter}
              found={found}
              empty={rows.length === 0}
              contentEmpty={
                byKind.session.length + byKind.message.length + byKind.file.length === 0
              }
            />
            {rows.flatMap(({ result, group, more }, i) => {
              const meta = GROUPS.find((g) => g.kind === group);
              const heading =
                group !== rows[i - 1]?.group ? (
                  <li key={`h-${group}`} className="secl" role="presentation">
                    {meta?.label}
                    {searching && <span className="c">{count(group)}</span>}
                  </li>
                ) : null;
              const showAll =
                more > 0 ? (
                  <li
                    data-item-id={`m-${group}`}
                    key={`m-${group}`}
                    className="pal-more"
                    role="presentation"
                  >
                    <Button
                      data-testid="shell.command-palette-set-filter"
                      type="button"
                      appearance="expander"
                      tabIndex={-1}
                      onClick={() => {
                        setFilter(group);
                      }}
                    >
                      Show all {count(group)}
                    </Button>
                  </li>
                ) : null;
              return [
                heading,
                <li
                  data-testid="shell.command-palette-run"
                  data-item-id={result.id}
                  key={result.id}
                  id={optionId(i)}
                  role="option"
                  aria-selected={i === active}
                  aria-label={`${result.title}, ${result.path.join(" › ")}${result.hint === undefined ? "" : `, ${result.hint}`}`}
                  data-kind={group}
                  onMouseMove={() => {
                    if (i !== active) select(i);
                  }}
                  onClick={() => {
                    run(result);
                  }}
                >
                  <Icon name={meta?.icon ?? "zap"} />
                  <span className="pr-main">
                    <span className="pr-title">
                      <Highlight text={result.title} query={text} />
                    </span>
                    <span className="pr-path">{result.path.join(" › ")}</span>
                  </span>
                  {result.hint !== undefined && <span className="k">{result.hint}</span>}
                </li>,
                showAll,
              ];
            })}
          </ul>
          <PalettePreview result={selected} query={text} />
        </div>
        {note !== null && (
          <p className="pal-note" role="status">
            {note}
          </p>
        )}
        <div className="palette-hint">
          <kbd>↑</kbd> <kbd>↓</kbd> move · <kbd>Enter</kbd> open
          {mode === "text" && (
            <>
              {" "}
              · <kbd>Tab</kbd> next type
            </>
          )}{" "}
          · <kbd>Esc</kbd> close
        </div>
      </div>
    </>
  );
}

function optionId(i: number): string {
  return `vw-palette-option-${String(i)}`;
}
