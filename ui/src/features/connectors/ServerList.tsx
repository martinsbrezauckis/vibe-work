/**
 * "Your MCP servers" on the Connectors page: the servers in Vibe Work's own Vibe config and
 * the ones the shown session knows (plugins, trusted project folders), each with its state; a click
 * opens the details. The "Add MCP server" button opens the add dialog.
 * Depends on: hooks, CategoryIcon, model/view. Called by: ConnectorsPage.
 */
import { builtinServerText } from "../../state/builtinToolsText";
import { Badge, Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { CategoryIcon } from "./CategoryIcon";
import { useConnectorActions, useConnectors } from "./hooks";
import { serverStatusText, serverTone } from "./model/view";

/** The servers section. */
export function ServerList(): ReactElement {
  const servers = useConnectors((s) => s.view?.servers ?? null);
  const selected = useConnectors((s) => s.selected);
  const actions = useConnectorActions();
  const list = servers ?? [];
  return (
    <section className="cgroup cservers" aria-labelledby="cservers-h">
      <div className="cgroup-head">
        <h3 id="cservers-h">Your MCP servers</h3>
        <Button
          data-testid="connectors.server-list-show-add"
          type="button"
          variant="primary"
          onClick={() => actions?.showAdd(true)}
        >
          Add MCP server
        </Button>
      </div>
      {list.length === 0 ? (
        <p className="cmuted">
          None yet. Add a server by its address: Vibe connects to it and signs in for you.
        </p>
      ) : (
        <ul className="srows">
          {list.map((s) => {
            const id = `s:${s.name}`;
            const tone = serverTone(s);
            const display = builtinServerText(s);
            return (
              <li data-item-id={s.name} key={s.name}>
                <button
                  data-testid="connectors.server-list-select"
                  type="button"
                  className={`srow ${selected === id ? "sel" : ""}`}
                  aria-pressed={selected === id}
                  data-server={s.name}
                  onClick={() => actions?.select(selected === id ? null : id)}
                >
                  <CategoryIcon name="server" />
                  <span className="ctile-text">
                    <span className="srow-name">{display?.name ?? s.displayName}</span>
                    {display ? <span className="ctile-line">{display.description}</span> : null}
                  </span>
                  <span className={`srow-addr${display ? "" : " mono"}`}>
                    {display ? (
                      <code>{s.name}</code>
                    ) : (
                      (s.address ?? (s.plugin === null ? "this computer" : `plugin ${s.plugin}`))
                    )}
                  </span>
                  <Badge presentation="connector-status" state={tone}>
                    {serverStatusText(s)}
                  </Badge>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
