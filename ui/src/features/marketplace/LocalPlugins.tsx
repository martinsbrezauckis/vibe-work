/** Plugins Vibe discovers outside the catalog, with their folder, contents and the open chat's pinned state.
 * Depends on marketplace hooks and the typed core view. Called by MarketplacePage.
 */
import { term } from "../../vocabulary";
import { PathLine } from "../../components/PathLine";
import { Button } from "../../components/Controls";
import { displayPluginName } from "../../state/pluginText";
import type { ReactElement } from "react";
import { useMarket, useMarketActions } from "./hooks";

/** The local plugin inventory. */
export function LocalPlugins(): ReactElement | null {
  const actions = useMarketActions();
  const view = useMarket((s) => s.view);
  const search = useMarket((s) => s.search);
  const filter = useMarket((s) => s.filter);
  if (view === null) return null;
  const plugins = view.discovered.filter(
    (p) =>
      filter !== "updates" &&
      `${p.name} ${p.description} ${p.path}`.toLowerCase().includes(search.trim().toLowerCase()),
  );
  if (plugins.length === 0 && !view.discoveryError && view.dropped.length === 0) return null;
  return (
    <section className="cgroup" aria-labelledby="local-plugins-heading" data-testid="local-plugins">
      <h3 id="local-plugins-heading">Plugins on this computer</h3>
      <p className="cmuted">
        Plugins Vibe found outside the marketplace. The ones that appear in Vibe Work's plugins
        folder stay off until you turn them on. When a plugin is on, its MCP servers can run code in
        every new chat.
      </p>
      {view.discoveryError ? (
        <p className="server-note" role="status">
          {view.discoveryError}
        </p>
      ) : null}
      {view.dropped.map((issue) => (
        <p className="server-note" role="status" key={`${issue.file}:${issue.message}`}>
          {issue.file}: {issue.message}
        </p>
      ))}
      <div className="cgrid">
        {plugins.map((p) => (
          <article key={p.path} data-item-id={p.path} className="local-plugin">
            <h4>
              <bdi>{displayPluginName(p.name)}</bdi>
            </h4>
            <p>{p.description || "No description provided."}</p>
            <p className="cmuted">
              {p.scope === "project" ? term("workNames", "projectTab").name : "This computer"} ·{" "}
              {p.off
                ? p.scope === "global"
                  ? "Off: found outside the marketplace"
                  : "Turned off"
                : p.available
                  ? "On for new chats"
                  : "Unavailable for new chats"}
            </p>
            <PathLine className="local-plugin-path" path={p.path} />
            <p className="cmuted">
              {p.inSession === null
                ? "Open a chat to see whether it runs there"
                : p.inSession.loaded
                  ? "Runs in this session"
                  : "Does not run in this session"}
            </p>
            <ul className="ctools">
              {p.components
                .filter((c) => c.kind !== "tool")
                .map((c) => (
                  <li key={`${c.kind}:${c.name}`}>
                    {c.kind === "skill" ? "Skill" : c.kind === "mcp_server" ? "MCP server" : c.kind}{" "}
                    · {c.name}
                    {c.kind === "mcp_server"
                      ? ` · ${String(p.servers.find((s) => s.name === c.name)?.tools ?? "Unknown")} tools`
                      : ""}
                  </li>
                ))}
            </ul>
            {[...new Set([...p.issues, ...(p.inSession?.issues ?? [])])].map((issue) => (
              <p className="server-note" role="status" key={issue}>
                {issue}
              </p>
            ))}
            {p.managed ? (
              <div className="cactions">
                <button
                  type="button"
                  className="tb-btn"
                  data-testid="marketplace.local-plugin-toggle"
                  onClick={() =>
                    void actions?.changePlugin(p.off ? "turnOnPlugin" : "turnOffPlugin", p.path)
                  }
                >
                  {p.off ? "Turn on" : "Turn off"}
                </button>
                <Button
                  type="button"
                  small
                  variant="danger"
                  data-testid="marketplace.local-plugin-remove"
                  onClick={() => void actions?.changePlugin("removePlugin", p.path)}
                >
                  Remove
                </Button>
              </div>
            ) : null}
            {!p.managed ? (
              <p className="cmuted">Managed elsewhere · change it in its own folder.</p>
            ) : null}
          </article>
        ))}
      </div>
    </section>
  );
}
