/**
 * The details panel of the selected marketplace entry: what it is and who makes it, what it installs and
 * runs, how it was reviewed, its licence and pin, its state in the viewed session (loaded, skills, Vibe's issues), and
 * its actions (install / update / stop / remove; for servers add, with the vendor's own rule when it only allows
 * approved apps and what to do instead).
 * Depends on: AppContext (link dialog), hooks, model, MarketCard (glyph, main action). Called by: MarketplacePage.
 * Invariants: vendor pages open only through the link dialog; the headless browser's note that it has none of the
 * built-in browser's address guards is the catalog's text, shown as is.
 */
import type { ReactElement } from "react";

import type { MarketEntryView, MarketView } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { CategoryIcon } from "../connectors/CategoryIcon";
import { glyphOf, mainAction } from "./MarketCard";
import { useMarket, useMarketActions, useServers } from "./hooks";
import { categoryLabel, contents, pinLine, progressText, statusOf } from "./model";

function SessionPart({ e }: { e: MarketEntryView }): ReactElement | null {
  const s = e.inSession;
  if (e.installed === null) return null;
  if (s === null) return <p className="cmuted">Open a session to see how it runs there.</p>;
  return (
    <>
      <h3>In the open session</h3>
      <p className="cmuted">
        {s.loaded
          ? "Loaded in this session."
          : "Not loaded in this session yet: new sessions use it."}
      </p>
      {s.skills.length === 0 ? null : (
        <ul className="ctools">
          {s.skills.map((k) => (
            <li key={k.name}>
              <span className="mono">/{k.name}</span>
              <span className="cmuted">
                {" "}
                {k.enabled === null ? "offered in new sessions" : k.enabled ? "on" : "off"}
              </span>
            </li>
          ))}
        </ul>
      )}
      {s.issues.map((issue) => (
        <p key={issue} className="cwarn">
          {issue}
        </p>
      ))}
      <p className="cmuted">
        Vibe has no switch for single plugin skills or for one session: remove the item to turn it
        off.
      </p>
    </>
  );
}

function ServerRule({ e }: { e: MarketEntryView }): ReactElement | null {
  const { transcript } = useApp();
  const server = e.entry.server;
  if (server === undefined) return null;
  return (
    <>
      <h3>Who can use it</h3>
      <p className="cmuted mono">{server.url}</p>
      {server.rule === undefined ? null : (
        <blockquote className="mrule">
          “{server.rule}”
          {server.ruleSource === undefined ? null : (
            <>
              {" "}
              <button
                type="button"
                className="linkish"
                onClick={() => {
                  transcript.askLink(server.ruleSource ?? null);
                }}
              >
                Vendor page
              </button>
            </>
          )}
        </blockquote>
      )}
      {server.whatToDo === undefined ? null : <p>{server.whatToDo}</p>}
      <p className="cmuted">Checked {server.checked}</p>
    </>
  );
}

function Actions({ e }: { e: MarketEntryView }): ReactElement {
  const actions = useMarketActions();
  const installing = useMarket((s) => s.installing);
  const servers = useServers();
  const action = mainAction(e, servers);
  const running = installing !== null && installing.id === e.entry.id;
  return (
    <div className="cactions">
      {action === "add" ? (
        <button
          type="button"
          className="btn-primary"
          onClick={() => void actions?.addServer(e.entry.id)}
        >
          Add and sign in
        </button>
      ) : null}
      {action === "addKey" ? (
        <button
          type="button"
          className="btn-primary"
          onClick={() => void actions?.addKeyServer(e.entry.id)}
        >
          Add with your key
        </button>
      ) : null}
      {action === "install" || action === "update" ? (
        <button
          type="button"
          className="btn-primary"
          disabled={installing !== null}
          onClick={() => void actions?.install(e.entry.id)}
        >
          {action === "install" ? "Install" : "Update"}
        </button>
      ) : null}
      {running ? (
        <button type="button" className="tb-btn" onClick={() => void actions?.cancel()}>
          Stop
        </button>
      ) : null}
      {e.installed !== null && !running ? (
        <>
          {e.installed.changed === true ? (
            <button
              type="button"
              className="tb-btn"
              onClick={() => void actions?.install(e.entry.id)}
            >
              Reinstall
            </button>
          ) : null}
          <button type="button" className="tb-btn" onClick={() => void actions?.remove(e.entry.id)}>
            Remove
          </button>
        </>
      ) : null}
    </div>
  );
}

function Body({ e, view }: { e: MarketEntryView; view: MarketView | null }): ReactElement {
  const { transcript } = useApp();
  const installing = useMarket((s) => s.installing);
  const servers = useServers();
  const status = statusOf(e, installing, servers);
  const pin = pinLine(e);
  const what = contents(e);
  const plugin = e.entry.plugin;
  return (
    <>
      <header className="cdetail-head">
        <span className="ctile-ico">
          <CategoryIcon name={glyphOf(e)} size={20} />
        </span>
        <div>
          <h2>{e.entry.name}</h2>
          <p className="cmuted">
            {e.entry.publisher} · {categoryLabel(view, e.entry.category)}
          </p>
        </div>
      </header>
      <p>{e.entry.summary}</p>
      <p>
        <span className={`cstat mstat-${status.tone}`}>{status.text}</span>
      </p>
      {installing !== null && installing.id === e.entry.id ? (
        <p className="cmuted" role="status">
          {progressText(installing)}
        </p>
      ) : null}
      {e.entry.notes.map((n) => (
        <p key={n} className={n.includes("address guards") ? "cwarn" : "cmuted"}>
          {n}
        </p>
      ))}
      {plugin === undefined ? null : (
        <>
          <h3>What it installs and runs</h3>
          <p className="cmuted">{plugin.consent}</p>
          <p className="cmuted">
            About {plugin.downloadMb} MB to download, {plugin.diskMb} MB on disk.
          </p>
        </>
      )}
      {e.entry.kind === "server" ? <ServerRule e={e} /> : null}
      <h3>Review and licence</h3>
      <p className="cmuted">{e.entry.review}</p>
      <p className="cmuted">
        Licence: {e.entry.licence}
        {what === "" ? "" : ` · ${what}`}
        {pin === null ? "" : ` · ${pin}`}
      </p>
      {e.entry.homepage === undefined ? null : (
        <p>
          <button
            type="button"
            className="linkish"
            onClick={() => {
              transcript.askLink(e.entry.homepage ?? null);
            }}
          >
            Vendor page
          </button>
        </p>
      )}
      <SessionPart e={e} />
      <Actions e={e} />
    </>
  );
}

/** The panel (nothing when no entry is selected). */
export function MarketDetail(): ReactElement | null {
  const view = useMarket((s) => s.view);
  const selected = useMarket((s) => s.selected);
  const actions = useMarketActions();
  const e = view?.entries.find((x) => x.entry.id === selected) ?? null;
  if (e === null) return null;
  return (
    <aside className="cdetail" aria-label={`${e.entry.name} details`} data-testid="market-detail">
      <button type="button" className="tb-btn cdetail-close" onClick={() => actions?.select(null)}>
        Close
      </button>
      <Body e={e} view={view} />
    </aside>
  );
}
