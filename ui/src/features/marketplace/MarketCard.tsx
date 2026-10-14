/**
 * One marketplace card: generic icon, name, publisher, one line, status in words, and the main action
 * (Install / Update / Add). A click on the card opens its details.
 * Depends on: CategoryIcon, model, hooks. Called by: MarketplacePage.
 * Invariants: no vendor logos; the status words come from `model` (catalog facts and Vibe's state).
 */
import type { ReactElement } from "react";

import type { MarketEntryView } from "../../protocol/core";
import { CategoryIcon, type GlyphName } from "../connectors/CategoryIcon";
import { useMarket, useMarketActions, useServers } from "./hooks";
import { addedServer, statusOf } from "./model";

/** Marketplace category -> one of the generic glyphs. */
export function glyphOf(e: MarketEntryView): GlyphName {
  if (e.entry.kind === "server") return "server";
  if (e.entry.kind === "plugin") return "plug";
  const map: Record<string, GlyphName> = {
    code: "code",
    docs: "docs",
    data: "data",
    search: "search",
    files: "files",
    work: "issues",
  };
  return map[e.entry.category] ?? "other";
}

/** The main action of an entry, or null. */
export function mainAction(
  e: MarketEntryView,
  servers: ReturnType<typeof useServers>,
): "install" | "update" | "add" | "addKey" | null {
  if (e.entry.kind === "server") {
    if (addedServer(e, servers) !== null) return null;
    // a server that needs an API key is added with the user's key (the add dialog, key part ticked).
    if (e.entry.server?.access === "key") return "addKey";
    return e.entry.server?.access === "signIn" ? "add" : null;
  }
  if (e.foreign || !e.installable) return null;
  if (e.installed === null) return "install";
  return e.installed.updateAvailable ? "update" : null;
}

const LABEL = { install: "Install", update: "Update", add: "Add", addKey: "Add with key" } as const;

/** A card. */
export function MarketCard({
  e,
  selected,
}: {
  e: MarketEntryView;
  selected: boolean;
}): ReactElement {
  const actions = useMarketActions();
  const installing = useMarket((s) => s.installing);
  const servers = useServers();
  const status = statusOf(e, installing, servers);
  const action = mainAction(e, servers);
  const busy = installing !== null;
  return (
    <div
      className={`ctile mcard ${selected ? "sel" : ""}`}
      data-entry={e.entry.id}
      data-kind={e.entry.kind}
    >
      <button
        type="button"
        className="ctile-main"
        aria-pressed={selected}
        onClick={() => actions?.select(selected ? null : e.entry.id)}
      >
        <span className="ctile-ico">
          <CategoryIcon name={glyphOf(e)} />
        </span>
        <span className="ctile-text">
          <span className="ctile-name">{e.entry.name}</span>
          <span className="ctile-line">{e.entry.summary}</span>
        </span>
      </button>
      <div className="ctile-foot">
        <span className={`cstat mstat-${status.tone}`} data-status={status.text}>
          {status.text}
        </span>
        <span className="ctile-via" title={e.entry.publisher}>
          {e.entry.publisher}
        </span>
        {action === null ? null : (
          <button
            type="button"
            className="tb-btn primary"
            disabled={busy && action !== "add" && action !== "addKey"}
            onClick={() => {
              if (action === "add") void actions?.addServer(e.entry.id);
              else if (action === "addKey") void actions?.addKeyServer(e.entry.id);
              else void actions?.install(e.entry.id);
            }}
          >
            {LABEL[action]}
          </button>
        )}
      </div>
    </div>
  );
}
