/**
 * "Your MCP servers" on the Connectors page: the servers in Vibe Work's own Vibe config and
 * the ones the shown session knows (plugins, trusted project folders), each with its state; a click
 * opens the details. The "Add MCP server" button opens the add dialog.
 * Depends on: hooks, CategoryIcon, model/view. Called by: ConnectorsPage.
 */
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
        <button type="button" className="tb-btn primary" onClick={() => actions?.showAdd(true)}>
          Add MCP server
        </button>
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
            return (
              <li key={s.name}>
                <button
                  type="button"
                  className={`srow ${selected === id ? "sel" : ""}`}
                  aria-pressed={selected === id}
                  data-server={s.name}
                  onClick={() => actions?.select(selected === id ? null : id)}
                >
                  <CategoryIcon name="server" />
                  <span className="srow-name">{s.displayName}</span>
                  <span className="srow-addr mono">
                    {s.address ?? (s.plugin === null ? "this computer" : `plugin ${s.plugin}`)}
                  </span>
                  <span className={`cstat ${tone}`}>{serverStatusText(s)}</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
