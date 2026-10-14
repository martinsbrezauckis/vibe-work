/**
 * The Settings page's column: the title, the search inside Settings (an "In: Settings"
 * chip says what it covers; clicking the chip, or Backspace in the empty field, carries the words to the global
 * search) and either the categories or, while a query is typed, the matching settings with their path.
 * Lives in the list column (`SessionList` renders it instead of the session list while Settings is open), so narrow
 * windows get it as the same drawer the session list uses.
 * Depends on: AppContext, hooks, catalog, components/Icon, components/Highlight. Called by: SessionList.
 * Invariants: no back arrow and no close × (leave through Home, any rail item or Esc); keyboard first: ↓ from
 * the field moves into the list, Enter on the field opens the first result, Esc clears the query (a second Esc goes
 * Home, handled by SettingsPage); a click on a result opens its category
 * and lights up the setting (the same action the global search's setting results use).
 */
import { type KeyboardEvent, type ReactElement, useRef } from "react";
import { openHit } from "./openHit";

import { Highlight } from "../../components/Highlight";
import { Icon } from "../../components/Icon";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";
import { useOfferedCategories } from "./availability";
import { searchSettings, settingPath } from "./catalog";

/** The id of the search field (SettingsPage focuses it on Ctrl F). */
export const SETTINGS_SEARCH_ID = "vw-settings-search";

/** The column. */
export function SettingsNav(): ReactElement {
  const app = useApp();
  const { shell } = app;
  const view = useShellState((s) => s.settings);
  const list = useRef<HTMLDivElement>(null);
  const offered = useOfferedCategories();
  const shownCategory = offered.find((c) => c.id === view.category) ?? offered[0];
  const results = searchSettings(view.query).filter((r) =>
    offered.some((c) => c.id === r.category),
  );
  const searching = view.query.trim() !== "";
  const widen = () => {
    const words = view.query;
    shell.actions.closeSettings();
    shell.actions.setOverlay("palette", words);
  };
  const onFieldKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape" && view.query !== "") {
      e.preventDefault();
      e.stopPropagation();
      shell.actions.setSettingsQuery("");
    } else if (e.key === "Enter" && results[0] !== undefined) {
      e.preventDefault();
      openHit(app, results[0]);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      list.current?.querySelector<HTMLButtonElement>("button")?.focus();
    } else if (e.key === "Backspace" && e.currentTarget.value === "") {
      e.preventDefault();
      widen();
    }
  };
  const onListKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const buttons = [...(list.current?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (at === -1) return;
    e.preventDefault();
    if (e.key === "ArrowUp" && at === 0) document.getElementById(SETTINGS_SEARCH_ID)?.focus();
    else
      buttons[
        Math.max(0, Math.min(buttons.length - 1, at + (e.key === "ArrowDown" ? 1 : -1)))
      ]?.focus();
  };
  return (
    <div className="set-nav">
      <h2 className="set-title">Settings</h2>
      <label className="set-search">
        <Icon name="search" size={14} />
        {/* The scope chip: a click (or Backspace in the empty field) widens the search to everything. No ×: Settings
            has no close or back controls at all ; leaving is Home, a rail item or Esc. */}
        <button
          type="button"
          className="scope-chip"
          aria-label="In: Settings. Search everything instead"
          title="Search everything instead (Backspace in the empty field)"
          onClick={(e) => {
            e.preventDefault();
            widen();
          }}
        >
          <span className="scope-in">In: Settings</span>
          <span className="scope-all" aria-hidden="true">
            Search all
          </span>
        </button>
        <input
          id={SETTINGS_SEARCH_ID}
          type="text"
          role="searchbox"
          aria-label="Search settings"
          placeholder="Search settings"
          autoComplete="off"
          value={view.query}
          onChange={(e) => {
            shell.actions.setSettingsQuery(e.target.value);
          }}
          onKeyDown={onFieldKey}
        />
      </label>
      {searching ? (
        <div
          className="set-results"
          ref={list}
          onKeyDown={onListKey}
          aria-label="Matching settings"
          role="group"
        >
          {results.length === 0 ? (
            <p className="col-note">No setting matches.</p>
          ) : (
            results.map((r) => (
              <button
                key={r.id}
                type="button"
                className="set-result"
                onClick={() => {
                  openHit(app, r);
                }}
              >
                <span className="set-result-label">
                  <Highlight text={r.label} query={view.query} />
                </span>
                <span className="set-result-path">{settingPath(r).join(" › ")}</span>
              </button>
            ))
          )}
        </div>
      ) : (
        <nav className="set-cats" aria-label="Settings categories" ref={list} onKeyDown={onListKey}>
          {offered.map((c) => (
            <button
              key={c.id}
              type="button"
              className="set-cat"
              aria-current={c.id === shownCategory?.id ? "page" : undefined}
              onClick={() => {
                shell.actions.setSettingsCategory(c.id);
              }}
            >
              <Icon name={c.icon} />
              {c.label}
            </button>
          ))}
        </nav>
      )}
    </div>
  );
}
