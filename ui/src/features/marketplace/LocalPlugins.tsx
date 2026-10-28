/** Plugins Vibe discovers outside the catalog, with their version, author, folder, contents and the open chat's
 * pinned state.
 * Depends on marketplace hooks and the typed core view. Called by MarketplacePage.
 */
import { term } from "../../vocabulary";
import { PathLine } from "../../components/PathLine";
import { Button, RowDisclosure } from "../../components/Controls";
import { builtinPluginText, builtinSkillText } from "../../state/builtinToolsText";
import { displayPluginName } from "../../state/pluginText";
import { type ReactElement, type ReactNode, useState } from "react";
import { useMarket, useMarketActions } from "./hooks";

/** Version and author as the plugin's manifest gives them: with the folder below, what tells plugins apart. */
function versionLine(version: string | null, author: string | null): string {
  if (version === null && author === null) return "No version or author given";
  const parts = [version === null ? "No version given" : `Version ${version}`];
  if (author !== null) parts.push(`by ${author}`);
  return parts.join(" · ");
}

/** "14 skills · 1 MCP server": what a folded contents list holds. */
function contentsLine(kinds: readonly string[]): string {
  const count = (kind: string) => kinds.filter((k) => k === kind).length;
  const part = (n: number, one: string, many: string) => `${String(n)} ${n === 1 ? one : many}`;
  const parts = [
    count("skill") > 0 ? part(count("skill"), "skill", "skills") : null,
    count("mcp_server") > 0 ? part(count("mcp_server"), "MCP server", "MCP servers") : null,
    kinds.length - count("skill") - count("mcp_server") > 0
      ? part(kinds.length - count("skill") - count("mcp_server"), "other part", "other parts")
      : null,
  ];
  return parts.filter((x) => x !== null).join(" · ");
}

/** A plugin's contents, folded until the person opens them (a long list pushed the other plugins away). */
function Contents({ kinds, children }: { kinds: readonly string[]; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  if (kinds.length === 0) return null;
  return (
    <>
      <RowDisclosure
        data-testid="marketplace.local-plugin-contents"
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
        }}
      >
        {contentsLine(kinds)}
      </RowDisclosure>
      {open && <ul className="ctools">{children}</ul>}
    </>
  );
}

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
      `${builtinPluginText(p.name, p.scope)?.name ?? ""} ${builtinPluginText(p.name, p.scope)?.description ?? ""} ${p.name} ${p.description} ${p.path}`
        .toLowerCase()
        .includes(search.trim().toLowerCase()),
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
              <bdi>{builtinPluginText(p.name, p.scope)?.name ?? displayPluginName(p.name)}</bdi>
            </h4>
            <p>
              {builtinPluginText(p.name, p.scope)?.description ??
                (p.description || "No description provided.")}
            </p>
            <p className="cmuted">
              {builtinPluginText(p.name, p.scope) ? (
                <>
                  <code>{p.name}</code> ·{" "}
                </>
              ) : null}
              {versionLine(p.version, p.author)}
            </p>
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
            <Contents kinds={p.components.filter((c) => c.kind !== "tool").map((c) => c.kind)}>
              {p.components
                .filter((c) => c.kind !== "tool")
                .map((c) => (
                  <li key={`${c.kind}:${c.name}`}>
                    {c.kind === "skill" ? "Skill" : c.kind === "mcp_server" ? "MCP server" : c.kind}{" "}
                    · {builtinSkillText({ name: c.name, source: p.scope })?.name ?? c.name}
                    {builtinSkillText({ name: c.name, source: p.scope }) ? (
                      <>
                        {" "}
                        · <code>{c.name}</code>
                      </>
                    ) : null}
                    {c.kind === "mcp_server"
                      ? ` · ${String(p.servers.find((s) => s.name === c.name)?.tools ?? "Unknown")} tools`
                      : ""}
                  </li>
                ))}
            </Contents>
            {[...new Set([...p.issues, ...(p.inSession?.issues ?? [])])].map((issue) => (
              <p className="server-note" role="status" key={issue}>
                {issue}
              </p>
            ))}
            {p.managed ? (
              <div className="cactions">
                <Button
                  type="button"

                  data-testid="marketplace.local-plugin-toggle"
                  onClick={() =>
                    void actions?.changePlugin(p.off ? "turnOnPlugin" : "turnOffPlugin", p.path)
                  }
                >
                  {p.off ? "Turn on" : "Turn off"}
                </Button>
                <Button
                  type="button"
                  shape="outlined"
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
