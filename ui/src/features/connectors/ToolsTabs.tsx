/**
 * The Tools page's tabs: Connectors (Mistral-hosted connectors and MCP servers) and
 * Marketplace (skill packs, plugins and MCP servers from the reviewed catalog).
 * Depends on: marketplace hooks. Called by: ConnectorsPage, MarketplacePage.
 */
import { Tabs, Tab } from "../../components/Tabs";
import type { ReactElement } from "react";

import { useMarket, useMarketActions } from "../marketplace/hooks";

const TABS = [
  { id: "connectors", label: "Connectors" },
  { id: "marketplace", label: "Marketplace" },
  { id: "skills", label: "Skills" },
] as const;

/** The tab strip. */
export function ToolsTabs(): ReactElement {
  const tab = useMarket((s) => s.tab);
  const actions = useMarketActions();
  return (
    <Tabs
      data-testid="connectors.tools-tabs-ttabs"
      variant="underline"

      aria-label="Tools"
    >
      {TABS.map((t) => (
        <Tab
          data-testid="connectors.tools-tabs-show-tab"
          data-item-id={t.id}
          key={t.id}
          type="button"

          aria-selected={tab === t.id}
          data-tab={t.id}
          onClick={() => void actions?.showTab(t.id)}
        >
          {t.label}
        </Tab>
      ))}
    </Tabs>
  );
}
