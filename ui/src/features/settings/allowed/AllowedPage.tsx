/**
 * Settings › Allowed without asking: the intro line, Find, the count line, the three groups, and the reading /
 * empty / no-match states; the bulk confirmation on top. Groups start collapsed above 10 entries, each list shows 10
 * rows then "Show 20 more", Find searches every entry (its words, detail and project folder name), and rows are in
 * one fixed order (last used first). No Sort by or Project selector.
 * Renders only: the store (allowed/store) holds the lists, the choices and every action; the view (allowed/view)
 * decides what shows.
 * Depends on: components (Controls, EmptyState), useAllowed, AllowedGroup, RevokeAllDialog.
 * Called by: SettingsPage (category `allowed`).
 */
import { type ReactElement, useId } from "react";

import { Button } from "../../../components/Controls";
import { EmptyState } from "../../../components/EmptyState";
import { AllowedGroup } from "./AllowedGroup";
import { RevokeAllDialog } from "./RevokeAllDialog";
import type { Allowed, AllowedState } from "./store";
import { useAllowed, useAllowedFocus, useSettled } from "./useAllowed";

/** Find (its words also match a project's folder name). */
function Find({ allowed, state }: { allowed: Allowed; state: AllowedState }): ReactElement {
  const id = useId();
  return (
    <div className="key-field allow-find">
      <label htmlFor={id}>Find</label>
      <div className="key-field-row">
        <input
          data-testid="settings.allowed-find"
          data-allowed-focus="find"
          id={id}
          type="text"
          placeholder="Command, site, tool, secret or project"
          autoComplete="off"
          value={state.ui.query}
          onChange={(e) => {
            allowed.setQuery(e.target.value);
          }}
        />
      </div>
    </div>
  );
}

/** The page body of the category. */
export function AllowedPage(): ReactElement {
  const { allowed, state, view } = useAllowed();
  useAllowedFocus();
  // While Find filters, "17 of 214 match" settles 300 ms after the last change (one announcement).
  const count = useSettled(view.countLine, view.filtering);
  return (
    <div className="allow-page">
      <p className="key-line">
        Everything Vibe may do without asking you, beyond your levels in Safety.
      </p>
      {view.state === "reading" ? (
        <p className="key-line" role="status">
          Reading what is allowed…
        </p>
      ) : view.state === "empty" ? (
        <EmptyState glyph="check">Nothing is allowed without asking.</EmptyState>
      ) : (
        <>
          <Find allowed={allowed} state={state} />
          <p className="key-line allow-count" role="status">
            {count}
          </p>
          {view.state === "noMatch" ? (
            <EmptyState glyph="check" small>
              <p>Nothing matches “{state.ui.query.trim()}”.</p>
              <Button
                data-testid="settings.allowed-clear-search"
                shape="outlined"
                onClick={() => {
                  allowed.clearSearch();
                }}
              >
                Clear the search
              </Button>
            </EmptyState>
          ) : (
            <div className="allow-groups">
              {view.groups.map((g) => (
                <AllowedGroup key={g.id} group={g} allowed={allowed} state={state} />
              ))}
            </div>
          )}
        </>
      )}
      <RevokeAllDialog allowed={allowed} state={state} />
    </div>
  );
}
