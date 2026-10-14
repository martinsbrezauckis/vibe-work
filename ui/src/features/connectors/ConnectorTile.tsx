/**
 * One connector tile: generic category icon, the name, one line, the status in words and,
 * for a connector that needs it, the Sign in button. A click on the tile opens its details.
 * Depends on: CategoryIcon, model/view, hooks. Called by: ConnectorsPage.
 * Invariant: no vendor logo; the status words are Vibe's states in plain language (`model/view`).
 */
import type { ReactElement } from "react";

import { CategoryIcon } from "./CategoryIcon";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useConnectorActions } from "./hooks";
import type { Tile } from "./model/view";

/** A tile. */
export function ConnectorTile({ tile, selected }: { tile: Tile; selected: boolean }): ReactElement {
  const actions = useConnectorActions();
  const keyBlocked = useKeyReadiness() !== null;
  return (
    <div
      className={`ctile ${selected ? "sel" : ""}`}
      data-alias={tile.alias}
      data-status={tile.status}
    >
      <button
        type="button"
        className="ctile-main"
        aria-pressed={selected}
        onClick={() => {
          actions?.select(selected ? null : `c:${tile.alias}`);
        }}
      >
        <span className="ctile-ico">
          <CategoryIcon name={tile.meta.category} />
        </span>
        <span className="ctile-text">
          <span className="ctile-name">{tile.meta.name}</span>
          <span className="ctile-line">{tile.meta.line}</span>
        </span>
      </button>
      <div className="ctile-foot">
        <span className={`cstat ${tile.status}`}>{tile.statusText}</span>
        <span className="ctile-via">via Mistral</span>
        {tile.meta.experimental === true ? (
          <span className="cstat">Experimental (Vibe)</span>
        ) : null}
        {tile.status === "signin" ? (
          <button
            type="button"
            className="tb-btn primary"
            disabled={keyBlocked}
            onClick={() => {
              void actions?.signIn(tile.alias);
            }}
          >
            Sign in
          </button>
        ) : null}
      </div>
    </div>
  );
}
