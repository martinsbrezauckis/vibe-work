/**
 * The details panel of the selected marketplace entry: what it is and who makes it, what it installs and
 * runs, how it was reviewed, its licence and pin, its state in the viewed session (loaded, skills, Vibe's issues), and
 * its actions (install / update / stop / remove; for servers add, with the vendor's own rule when it only allows
 * approved apps and what to do instead). An install shows the offer block directly above Install, which
 * then installs exactly that offer.
 * Depends on: AppContext (link dialog), hooks, model, MarketCard (glyph, main action), InstallOffer.
 * Called by: MarketplacePage.
 * Invariants: vendor pages open only through the link dialog; the headless browser's note that it has none of the
 * built-in browser's address guards is the catalog's text, shown as is.
 */
import { Badge, Button } from "../../components/Controls";
import { type ReactElement, useRef } from "react";

import type { MarketEntryView, MarketView } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { CategoryIcon } from "../connectors/CategoryIcon";
import { escapeCloses, InstallOffer, useOfferFocus } from "../connectors/InstallOffer";
import { glyphOf, mainAction } from "./MarketCard";
import { useMarket, useMarketActions, useMarketOffers, useServers } from "./hooks";
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
        <p key={issue} className="server-note" role="status">
          {issue}
        </p>
      ))}
      <p className="cmuted">
        Vibe has no switch for single plugin skills or for one session: turn off the plugin to stop
        it in new chats.
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
                data-testid="marketplace.market-detail-ask-link"
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
  const offer = useMarketOffers((o) => o.open[e.entry.id]);
  const focus = useMarketOffers((o) => o.focus);
  const confirm = useRef<HTMLButtonElement>(null);
  useOfferFocus(focus, e.entry.id, "confirm", confirm);
  const cancel = () => {
    actions?.cancelOffer(e.entry.id);
  };
  return (
    <div className="cactions">
      {action === "add" ? (
        <Button
          data-testid="marketplace.market-detail-add-server"
          type="button"
          appearance="dialog"
          onClick={() => void actions?.addServer(e.entry.id)}
        >
          Add and sign in
        </Button>
      ) : null}
      {action === "addKey" ? (
        <Button
          data-testid="marketplace.market-detail-add-key-server"
          type="button"
          appearance="dialog"
          onClick={() => void actions?.addKeyServer(e.entry.id)}
        >
          Add with your key
        </Button>
      ) : null}
      {action === "install" || action === "update" ? (
        <Button
          data-testid="marketplace.market-detail-install"
          ref={confirm}
          type="button"
          appearance="dialog"
          disabled={installing !== null || (offer !== undefined && offer.phase !== "shown")}
          onClick={() =>
            void (action === "install"
              ? actions?.installShown(e.entry.id)
              : actions?.install(e.entry.id))
          }
          onKeyDown={offer === undefined ? undefined : escapeCloses(cancel)}
        >
          {action === "update"
            ? "Update"
            : offer?.phase === "installing"
              ? "Installing…"
              : "Install"}
        </Button>
      ) : null}
      {running ? (
        <Button
          data-testid="marketplace.market-detail-cancel"
          type="button"

          onClick={() => void actions?.cancel()}
        >
          Stop
        </Button>
      ) : null}
      {e.installed !== null && !running ? (
        <>
          {e.installed.changed === true ? (
            <Button
              data-testid="marketplace.market-detail-install-2"
              type="button"

              onClick={() => void actions?.install(e.entry.id)}
            >
              Reinstall
            </Button>
          ) : null}
          {e.folder ? (
            <Button
              type="button"

              data-testid="marketplace.market-detail-toggle"
              onClick={() => {
                if (e.folder)
                  void actions?.changePlugin(e.off ? "turnOnPlugin" : "turnOffPlugin", e.folder);
              }}
            >
              {e.off ? "Turn on" : "Turn off"}
            </Button>
          ) : null}
          <Button
            data-testid="marketplace.market-detail-remove"
            type="button"
            shape="outlined"
            variant="danger"
            onClick={() =>
              e.off && e.folder
                ? void actions?.changePlugin("removePlugin", e.folder)
                : void actions?.remove(e.entry.id)
            }
          >
            Remove
          </Button>
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
        <Badge presentation="market-status" state={status.tone}>
          {status.text}
        </Badge>
      </p>
      {installing !== null && installing.id === e.entry.id ? (
        <p className="cmuted" role="status">
          {progressText(installing)}
        </p>
      ) : null}
      {e.entry.notes.map((n) => (
        <p
          key={n}
          role="status"
          className={n.includes("address guards") ? "server-note" : "cmuted"}
        >
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
            data-testid="marketplace.market-detail-ask-link-2"
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
      <Offer e={e} />
      <Actions e={e} />
    </>
  );
}

/** The install offer block right above Install, and the read-failed line when the offer could not be read. */
function Offer({ e }: { e: MarketEntryView }): ReactElement | null {
  const actions = useMarketActions();
  const offer = useMarketOffers((o) => o.open[e.entry.id]);
  const problem = useMarketOffers((o) => o.problems[e.entry.id]);
  const focus = useMarketOffers((o) => o.focus);
  return (
    <>
      {problem === undefined ? null : (
        <p className="key-problem" role="alert" data-testid="marketplace.offer-problem">
          {problem}
        </p>
      )}
      {offer === undefined ? null : (
        <InstallOffer
          testId="marketplace.offer"
          open={offer}
          focus={focus}
          onCancel={() => {
            actions?.cancelOffer(e.entry.id);
          }}
        />
      )}
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
      <Button
        data-testid="marketplace.market-detail-select"
        type="button"
        placement="detail-close"
        onClick={() => actions?.select(null)}
      >
        Close
      </Button>
      <Body e={e} view={view} />
    </aside>
  );
}
