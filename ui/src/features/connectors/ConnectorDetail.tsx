/**
 * The details panel of the selected connector or server: what it is, its state with Vibe's
 * own reason, the tools Vibe reports, who keeps the sign-in, and its actions (sign in, turn off or on,
 * manage at Mistral; for servers also sign out and remove).
 * Depends on: AppContext (link dialog), hooks, CategoryIcon, model/meta, model/view. Called by:
 * ConnectorsPage.
 * Invariants: links (Mistral's console) open only through the link dialog; plugin servers are shown, not
 * changed here (their plugin owns them).
 */
import { builtinServerText } from "../../state/builtinToolsText";
import { Badge, Button } from "../../components/Controls";
import type { ReactElement } from "react";

import type { ServerView, ToolView } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { CategoryIcon } from "./CategoryIcon";
import { useConnectorActions, useConnectors } from "./hooks";
import { CATEGORIES } from "./model/meta";
import { type Tile, serverStatusText, serverTone, tileOf } from "./model/view";

function Tools({ tools, empty }: { tools: readonly ToolView[]; empty: string }): ReactElement {
  if (tools.length === 0) return <p className="cmuted">{empty}</p>;
  return (
    <ul className="ctools">
      {tools.map((t) => (
        <li key={t.name} className={t.enabled === false ? "off" : ""}>
          <span className="mono">{t.name}</span>
          {t.description === null ? null : <span className="cmuted"> {t.description}</span>}
        </li>
      ))}
    </ul>
  );
}

function ConnectorPart({
  tile,
  manageUrl,
}: {
  tile: Tile;
  manageUrl: string | null;
}): ReactElement {
  const actions = useConnectorActions();
  const keyBlocked = useKeyReadiness() !== null;
  const { transcript } = useApp();
  const category = CATEGORIES.find((c) => c.id === tile.meta.category)?.label ?? "";
  return (
    <>
      <header className="cdetail-head">
        <span className="ctile-ico">
          <CategoryIcon name={tile.meta.category} size={20} />
        </span>
        <div>
          <h2>{tile.meta.name}</h2>
          <p className="cmuted">{category} · via Mistral</p>
        </div>
      </header>
      <p>{tile.meta.line}</p>
      <p>
        <Badge presentation="connector-status" state={tile.status}>
          {tile.statusText}
        </Badge>
        {tile.reason === null ? null : <span className="creason"> {tile.reason}</span>}
      </p>
      {tile.status === "setup" ? (
        <p className="cmuted">
          This connector needs its setup finished in Mistral's console first.
        </p>
      ) : null}
      <h3>What it can do</h3>
      <Tools tools={tile.connector.tools} empty="Mistral lists its tools once you are signed in." />
      <h3>Sign-in and access</h3>
      <p className="cmuted">
        Mistral keeps this sign-in (it never reaches this computer). Sessions use the connector in
        their own access mode, like every other tool.
      </p>
      <div className="cactions">
        {tile.status === "signin" ? (
          <Button
            data-testid="connectors.connector-detail-sign-in"
            type="button"
            appearance="dialog"
            disabled={keyBlocked}
            onClick={() => void actions?.signIn(tile.alias)}
          >
            Sign in
          </Button>
        ) : null}
        <Button
          data-testid="connectors.connector-detail-turn-off"
          type="button"

          disabled={keyBlocked}
          onClick={() => void actions?.turnOff(tile.alias, tile.status !== "off", tile.meta.name)}
        >
          {tile.status === "off" ? "Turn on" : "Turn off"}
        </Button>
        {manageUrl === null ? null : (
          <Button
            data-testid="connectors.connector-detail-ask-link"
            type="button"

            onClick={() => {
              transcript.askLink(manageUrl);
            }}
          >
            Manage at Mistral
          </Button>
        )}
      </div>
      {manageUrl === null ? (
        <p className="cmuted">
          To remove the account link itself, use the connectors page of your Mistral console.
        </p>
      ) : null}
    </>
  );
}

/**: which key a key server gets, the grant, and "Allow key" when it is not allowed now. */
function KeyLine({ server }: { server: ServerView }): ReactElement {
  const actions = useConnectorActions();
  const vaultBlocked = useKeyReadiness("vault") !== null;
  const key = server.key;
  if (key === null) return <></>;
  const grant =
    key.grant === "always"
      ? "allowed always (until you revoke it in Settings, Project secrets)"
      : key.grant === "untilClose"
        ? "allowed until Vibe Work closes"
        : key.grant === "locked"
          ? "your vault is locked"
          : "not allowed now";
  return (
    <>
      <p className="cmuted" data-key-grant={key.grant}>
        {key.secretName === null
          ? "Uses a key from your vault"
          : `Uses your secret “${key.secretName}” in ${key.header ?? "a header"}`}
        : {grant}. Vibe Work adds it to each request; Vibe and the agent never see it.
      </p>
      {(key.grant === "none" || key.grant === "locked") && vaultBlocked ? (
        <KeyNotice
          requirement="vault"
          message="MCP server keys use your vault to keep their values encrypted."
        />
      ) : null}
      {key.grant === "none" ? (
        <div className="cactions">
          <Button
            data-testid="connectors.connector-detail-allow-server-key"
            type="button"
            appearance="dialog"
            disabled={vaultBlocked}
            onClick={() => void actions?.allowServerKey(server.name)}
          >
            Allow key
          </Button>
        </div>
      ) : null}
    </>
  );
}

function ServerPart({ server }: { server: ServerView }): ReactElement {
  const actions = useConnectorActions();
  const tone = serverTone(server);
  const display = builtinServerText(server);
  return (
    <>
      <header className="cdetail-head">
        <span className="ctile-ico">
          <CategoryIcon name="server" size={20} />
        </span>
        <div>
          <h2>{display?.name ?? server.displayName}</h2>
          <p className="cmuted mono">{server.address ?? "Runs on this computer"}</p>
        </div>
      </header>
      <p>
        <Badge presentation="connector-status" state={tone}>
          {serverStatusText(server)}
        </Badge>
        {server.error === null ? null : <span className="creason"> {server.error}</span>}
      </p>
      {display ? (
        <>
          <p>{display.description}</p>
          <p className="cmuted">
            <code>{server.name}</code>
            {server.plugin === null ? null : <> · {server.plugin}</>}
          </p>
        </>
      ) : null}
      <h3>What it can do</h3>
      <Tools tools={server.tools} empty="Its tools show once a session has connected to it." />
      <h3>Sign-in</h3>
      {server.key !== null ? (
        <KeyLine server={server} />
      ) : (
        <p className="cmuted">
          {server.signIn === "oauth"
            ? "Vibe signs in for you; the token stays in this computer's keychain (Vibe's), never in Vibe Work."
            : server.signIn === "static"
              ? "Signs in with a key or header in Vibe's config (set up outside Vibe Work)."
              : "No sign-in."}
        </p>
      )}
      {server.plugin !== null ? (
        <p className="cmuted">Comes with the plugin {server.plugin}; it is managed there.</p>
      ) : !server.configured ? (
        <p className="cmuted">
          Configured outside Vibe Work's own settings (for example in a project folder).
        </p>
      ) : (
        <div className="cactions">
          {server.signIn === "oauth" && tone !== "connected" ? (
            <Button
              data-testid="connectors.connector-detail-sign-in-server"
              type="button"
              appearance="dialog"
              onClick={() => void actions?.signInServer(server.name)}
            >
              Sign in
            </Button>
          ) : null}
          {server.signIn === "oauth" && tone === "connected" ? (
            <Button
              data-testid="connectors.connector-detail-sign-out-server"
              type="button"

              onClick={() => void actions?.signOutServer(server.name)}
            >
              Sign out
            </Button>
          ) : null}
          <Button
            data-testid="connectors.connector-detail-set-server-off"
            type="button"

            onClick={() => void actions?.setServerOff(server.name, !server.off)}
          >
            {server.off ? "Turn on" : "Turn off"}
          </Button>
          <Button
            data-testid="connectors.connector-detail-remove-server"
            type="button"
            shape="outlined"
            variant="danger"
            onClick={() => actions?.askRemoveServer(server.name)}
          >
            Remove
          </Button>
        </div>
      )}
    </>
  );
}

/** The panel for the selected tile or server, or nothing. */
export function ConnectorDetail(): ReactElement | null {
  const selected = useConnectors((s) => s.selected);
  const view = useConnectors((s) => s.view);
  const actions = useConnectorActions();
  if (selected === null || view === null) return null;
  const kind = selected.slice(0, 2);
  const id = selected.slice(2);
  const connector = kind === "c:" ? view.connectors.find((c) => c.alias === id) : undefined;
  const server = kind === "s:" ? view.servers.find((s) => s.name === id) : undefined;
  if (connector === undefined && server === undefined) return null;
  return (
    <aside className="cdetail" aria-label="Details">
      <Button
        data-testid="connectors.connector-detail-select"
        type="button"
        placement="detail-close"
        onClick={() => actions?.select(null)}
      >
        Close
      </Button>
      {connector !== undefined ? (
        <ConnectorPart tile={tileOf(connector)} manageUrl={view.manageUrl} />
      ) : server !== undefined ? (
        <ServerPart server={server} />
      ) : null}
    </aside>
  );
}
