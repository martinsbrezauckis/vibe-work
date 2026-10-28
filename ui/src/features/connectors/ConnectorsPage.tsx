/**
 * The Connectors page: Mistral-hosted connectors grouped by
 * category with search and filters, each with its state in plain words; the sign-in in progress; "Your
 * MCP servers" with "Add MCP server"; the details panel; the link dialog for sign-in pages and Mistral's
 * console. Shown in the content area from the rail's Tools button.
 * Depends on: hooks, the connector views, model/view, the transcript's LinkDialog. Called by: Content.
 * Invariants: components render, the connectors controller decides; no vendor logos.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { Selector } from "../../components/Selector";
import { LinkDialog } from "../transcript/LinkDialog";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { AddServerDialog } from "./AddServerDialog";
import { RemoveServerDialog } from "./RemoveServerDialog";
import { ConnectorDetail } from "./ConnectorDetail";
import { ConnectorTile } from "./ConnectorTile";
import { ToolsTabs } from "./ToolsTabs";
import { ServerList } from "./ServerList";
import { SignInBanner } from "./SignInBanner";
import { useConnectorActions, useConnectors } from "./hooks";
import { FILTERS, counts, groups } from "./model/view";

function CatalogNote(): ReactElement | null {
  const keyBlocked = useKeyReadiness() !== null;
  const view = useConnectors((s) => s.view);
  const error = useConnectors((s) => s.error);
  const loading = useConnectors((s) => s.loading);
  const text =
    (keyBlocked ? null : error) ??
    (view === null
      ? loading
        ? "Reading your connectors…"
        : null
      : view.catalog === "notLoaded" && !keyBlocked
        ? `Mistral's connector list could not be loaded${view.catalogError === null ? "." : `: ${view.catalogError}`}`
        : null);
  return text === null ? null : (
    <p className="server-note" role="status">
      <span className="px warn" aria-hidden="true" />
      <span>{text}</span>
    </p>
  );
}

function Toolbar(): ReactElement {
  const keyBlocked = useKeyReadiness() !== null;
  const actions = useConnectorActions();
  const filter = useConnectors((s) => s.filter);
  const search = useConnectors((s) => s.search);
  const view = useConnectors((s) => s.view);
  const n = counts(view);
  return (
    <div className="cbar">
      <label className="csearch">
        <input
          data-testid="connectors.connectors-page-set-search"
          value={search}
          placeholder="Search connectors"
          aria-label="Search connectors"
          onChange={(e) => actions?.setSearch(e.target.value)}
        />
      </label>
      <Selector
        testId="connectors.connectors-page-set-filter"
        label="Filter connectors"
        value={filter}
        options={FILTERS.map((f) => ({ value: f.id, label: `${f.label} ${String(n[f.id])}` }))}
        onChange={(value) => actions?.setFilter(value)}
        compact
      />
      <Button
        data-testid="connectors.connectors-page-refresh"
        type="button"

        disabled={keyBlocked}
        onClick={() => void actions?.refresh()}
      >
        Refresh
      </Button>
    </div>
  );
}

/** The page. */
export function ConnectorsPage(): ReactElement {
  const view = useConnectors((s) => s.view);
  const filter = useConnectors((s) => s.filter);
  const search = useConnectors((s) => s.search);
  const selected = useConnectors((s) => s.selected);
  const addOpen = useConnectors((s) => s.addOpen);
  const note = useConnectors((s) => s.note);
  const shown = groups(view, filter, search);
  return (
    <div className={`cpage ${selected === null ? "" : "has-detail"}`} data-testid="connectors-page">
      <div className="cmain">
        <div className="chead">
          <ToolsTabs />
          <p className="cmuted">
            Mistral hosts these and keeps their sign-ins. Sessions use them in their own access
            mode.
          </p>
        </div>
        <Toolbar />
        <KeyNotice message="Mistral's hosted connectors use your Mistral key to load and manage your connected accounts." />
        <CatalogNote />
        {view?.privateConnectorsHidden === true && (
          <p className="cnote" role="status">
            This key can use shared connectors only.
          </p>
        )}
        <SignInBanner />
        {note === null ? null : (
          <p className="cnote" role="status">
            {note}
          </p>
        )}
        {shown.map((g) => (
          <section
            key={g.id}
            className="cgroup"
            aria-labelledby={`cg-${g.id}`}
            data-category={g.id}
          >
            <h3 id={`cg-${g.id}`}>{g.label}</h3>
            <div className="cgrid">
              {g.tiles.map((t) => (
                <ConnectorTile key={t.alias} tile={t} selected={selected === `c:${t.alias}`} />
              ))}
            </div>
          </section>
        ))}
        {view !== null && shown.length === 0 && view.connectors.length > 0 ? (
          <p className="cmuted">
            Nothing here matches. If the tool runs an MCP server, add it by its address.
          </p>
        ) : null}
        <ServerList />
      </div>
      <ConnectorDetail />
      <AddServerDialog open={addOpen} />
      <RemoveServerDialog />
      <LinkDialog />
    </div>
  );
}
