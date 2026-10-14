/**
 * Hooks of the Marketplace tab: its state from the marketplace controller and its actions, plus the
 * Connectors view's servers (a catalog server's live state). Without a core the state is a fixed initial one and the
 * actions are absent.
 * Depends on: AppContext, createStore, state/market, connectors hooks. Called by: the marketplace views, ToolsTabs.
 */
import { useApp } from "../shell/AppContext";
import { createStore, useStore } from "../../state/createStore";
import { MARKET_INITIAL, type Market, type MarketState } from "../../state/market";
import type { ServerView } from "../../protocol/core";
import { useConnectors } from "../connectors/hooks";

const NONE = createStore<MarketState>(MARKET_INITIAL);
const NO_SERVERS: readonly ServerView[] = [];

/** Subscribes to `select(state)` of the tab (see `useStore` for the selector rule). */
export function useMarket<S>(select: (state: MarketState) => S): S {
  const { market } = useApp();
  return useStore(market?.store ?? NONE, select);
}

/** The window's own actions, or `null` without a core. */
export function useMarketActions(): Market["actions"] | null {
  return useApp().market?.actions ?? null;
}

/** The MCP servers Vibe has (from the Connectors view). */
export function useServers(): readonly ServerView[] {
  return useConnectors((s) => s.view?.servers ?? NO_SERVERS);
}
