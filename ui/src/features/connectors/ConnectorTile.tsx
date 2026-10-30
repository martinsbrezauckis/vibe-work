/**
 * One connector tile: generic category icon, the name, one line, the status in words and,
 * for a connector that needs it, the Sign in or (no sign-in needed) the Connect button. A click on the tile opens its details.
 * Depends on: CategoryIcon, model/view, hooks. Called by: ConnectorsPage.
 * Invariant: no vendor logo; the status words are Vibe's states in plain language (`model/view`).
 */
import { Card } from "../../components/Card";
import { Badge, Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { CategoryIcon } from "./CategoryIcon";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useConnectorActions } from "./hooks";
import type { Tile } from "./model/view";
import { TEXT } from "../../vocabulary";

/** A tile. */
export function ConnectorTile({ tile, selected }: { tile: Tile; selected: boolean }): ReactElement {
  const actions = useConnectorActions();
  const keyBlocked = useKeyReadiness() !== null;
  return (
    <Card
      variant="connector"
      data-item-id={tile.alias}
      selected={selected}
      data-alias={tile.alias}
      data-status={tile.status}
    >
      <button
        data-testid="connectors.connector-tile-select"
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
        <Badge presentation="connector-status" state={tile.status}>
          {tile.statusText}
        </Badge>
        <span className="ctile-via">via Mistral</span>
        {tile.meta.experimental === true ? (
          <Badge presentation="connector-status">Experimental (Vibe)</Badge>
        ) : null}
        {tile.status === "signin" ? (
          <Button
            data-testid="connectors.connector-tile-sign-in"
            type="button"
            variant="primary"
            disabled={keyBlocked}
            onClick={() => {
              void actions?.signIn(tile.alias);
            }}
          >
            Sign in
          </Button>
        ) : null}
        {tile.status === "connect" ? (
          <Button
            data-testid="connectors.connector-tile-connect"
            type="button"
            variant="primary"
            disabled={keyBlocked}
            onClick={() => {
              void actions?.connect(tile.alias, tile.meta.name);
            }}
          >
            {TEXT.connectorConnect}
          </Button>
        ) : null}
      </div>
    </Card>
  );
}
