/**
 * The Marketplace tab of the Tools page: plugins, skill packs and MCP servers from the reviewed catalog,
 * with search, filters, the install in progress and the details panel.
 * Depends on: hooks, model, MarketCard, MarketDetail, ToolsTabs. Called by: Content.
 * Invariants: components render, the marketplace controller decides; no vendor logos.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { Selector } from "../../components/Selector";
import { ToolsTabs } from "../connectors/ToolsTabs";
import { LocalPlugins } from "./LocalPlugins";
import { MarketCard } from "./MarketCard";
import { MarketDetail } from "./MarketDetail";
import { useMarket, useMarketActions, useServers } from "./hooks";
import { MARKET_FILTERS, marketCounts, marketGroups, progressText } from "./model";

function Toolbar(): ReactElement {
  const actions = useMarketActions();
  const filter = useMarket((s) => s.filter);
  const search = useMarket((s) => s.search);
  const view = useMarket((s) => s.view);
  const n = marketCounts(view, useServers());
  return (
    <div className="cbar">
      <label className="csearch">
        <input
          data-testid="marketplace.marketplace-page-set-search"
          value={search}
          placeholder="Search the marketplace"
          aria-label="Search the marketplace"
          onChange={(e) => actions?.setSearch(e.target.value)}
        />
      </label>
      <Selector
        testId="marketplace.marketplace-page-set-filter"
        label="Filter marketplace"
        value={filter}
        options={MARKET_FILTERS.map((f) => ({
          value: f.id,
          label: `${f.label} ${String(n[f.id])}`,
        }))}
        onChange={(value) => actions?.setFilter(value)}
        compact
      />
    </div>
  );
}

function Progress(): ReactElement | null {
  const installing = useMarket((s) => s.installing);
  const view = useMarket((s) => s.view);
  const actions = useMarketActions();
  if (installing === null) return null;
  const name = view?.entries.find((e) => e.entry.id === installing.id)?.entry.name ?? installing.id;
  return (
    <div className="csignin" role="status" data-testid="market-progress">
      <div className="csignin-row">
        <span>
          Installing {name}: {progressText(installing)}
        </span>
        <div className="csignin-acts">
          <Button
            data-testid="marketplace.marketplace-page-cancel"
            type="button"

            onClick={() => void actions?.cancel()}
          >
            Stop
          </Button>
        </div>
      </div>
    </div>
  );
}

/** The tab. */
export function MarketplacePage(): ReactElement {
  const view = useMarket((s) => s.view);
  const filter = useMarket((s) => s.filter);
  const search = useMarket((s) => s.search);
  const selected = useMarket((s) => s.selected);
  const note = useMarket((s) => s.note);
  const error = useMarket((s) => s.error);
  const loading = useMarket((s) => s.loading);
  const servers = useServers();
  const shown = marketGroups(view, filter, search, servers);
  const problem = error ?? view?.error ?? null;
  return (
    <div
      className={`cpage ${selected === null ? "" : "has-detail"}`}
      data-testid="marketplace-page"
    >
      <div className="cmain">
        <div className="chead">
          <ToolsTabs />
          <p className="cmuted">
            Install reviewed tools from the marketplace, and see the plugins Vibe finds on this
            computer.
          </p>
        </div>
        <Toolbar />
        {problem === null ? null : (
          <p className="server-note" role="status">
            <span className="px warn" aria-hidden="true" />
            <span>{problem}</span>
          </p>
        )}
        {view === null && loading ? <p className="cmuted">Reading the marketplace…</p> : null}
        <Progress />
        {note === null ? null : (
          <p className="cnote" role="status">
            {note}
          </p>
        )}
        <LocalPlugins />
        {shown.map((g) => (
          <section key={g.id} className="cgroup" aria-labelledby={`mg-${g.id}`} data-group={g.id}>
            <h3 id={`mg-${g.id}`}>{g.label}</h3>
            <p className="cmuted">{g.line}</p>
            <div className="cgrid">
              {g.entries.map((e) => (
                <MarketCard key={e.entry.id} e={e} selected={selected === e.entry.id} />
              ))}
            </div>
          </section>
        ))}
        {view !== null && shown.length === 0 && view.discovered.length === 0 ? (
          <p className="cmuted">Nothing here matches.</p>
        ) : null}
      </div>
      <MarketDetail />
    </div>
  );
}
