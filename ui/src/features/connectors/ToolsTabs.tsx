/**
 * The Tools page's two tabs: Connectors (Mistral-hosted connectors and MCP servers) and
 * Marketplace (skill packs, plugins and MCP servers from the reviewed catalog).
 * Depends on: marketplace hooks. Called by: ConnectorsPage, MarketplacePage.
 */
import type { ReactElement } from "react";

import { useMarket, useMarketActions } from "../marketplace/hooks";

const TABS = [
  { id: "connectors", label: "Connectors" },
  { id: "marketplace", label: "Marketplace" },
] as const;

/** The tab strip. */
export function ToolsTabs(): ReactElement {
  const tab = useMarket((s) => s.tab);
  const actions = useMarketActions();
  return (
    <div
      data-testid="connectors.tools-tabs-ttabs"
      className="ttabs"
      role="tablist"
      aria-label="Tools"
    >
      {TABS.map((t) => (
        <button
          data-testid="connectors.tools-tabs-show-tab"
          data-item-id={t.id}
          key={t.id}
          type="button"
          role="tab"
          className="ttab"
          aria-selected={tab === t.id}
          data-tab={t.id}
          onClick={() => void actions?.showTab(t.id)}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}
