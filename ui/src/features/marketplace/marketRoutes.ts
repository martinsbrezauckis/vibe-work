/**
 * The `ui/marketplace` UI-control route. The Rust side lists the route name
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES). Every action calls the same function a click calls, through the Debug
 * API's action set (`app.market.debugApi`), so every core call carries the Debug API origin (the Debug
 * API acts as the user; the core asks it no dialog):
 * - `read` (default): tab, filter, search, selection, note, error, the install in progress, platform, session, and
 *   each shown entry (id, kind, name, publisher, status words, installed version, update, in-session state);
 * - `open` (shows the Marketplace tab), `connectors` (shows the Connectors tab), `filter {filter}`, `search {text}`,
 *   `select {id|null}`, `reload`;
 * - `install {id}` (also updates; without an offer), `cancel`, `remove {id}`, `addServer {id}` (a catalog server
 *   through the Connectors flow; its sign-in finishes only in a browser);
 * - the one-click install: `offer {id}` = the card's Install… (selects the card, shows the offer in the
 *   detail), `installOffer {id}` = the detail's Install (installs exactly the shown offer; without one it shows the
 *   offer first, like the click), `cancelOffer {id}` = Escape on the offer; `read` lists them under `offers`;
 * - `turnOnPlugin {path}`, `turnOffPlugin {path}`, `removePlugin {path}`: a plugin Vibe found (`discovered` in `read`),
 *   also one not installed from the catalog.
 * Depends on: AppContext, uiControl, model. Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`).
 */
import { toCallError } from "../../protocol/client/types";
import { offersReport } from "../../state/installOffers";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { MARKET_FILTERS, type MarketFilter, marketGroups, statusOf } from "./model";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(input: Record<string, unknown>, key: string): string {
  const value = input[key];
  if (typeof value === "string" && value !== "") return value;
  throw new UiControlError("invalidParams", `${key} must be a non-empty string`);
}

function read(app: App): unknown {
  const s = app.market?.store.get();
  if (s === undefined) return null;
  const servers = app.connectors?.store.get().view?.servers ?? [];
  return {
    open: app.connectors?.store.get().open ?? false,
    tab: s.tab,
    filter: s.filter,
    search: s.search,
    selected: s.selected,
    note: s.note,
    error: s.error ?? s.view?.error ?? null,
    installing: s.installing,
    platform: s.view?.platform ?? null,
    sessionId: s.view?.sessionId ?? null,
    catalogVersion: s.view?.catalogVersion ?? null,
    discovered: s.view?.discovered ?? [],
    offers: app.market === null ? null : offersReport(app.market.offers.get()),
    discoveryError: s.view?.discoveryError ?? null,
    groups: marketGroups(s.view, s.filter, s.search, servers).map((g) => ({
      group: g.id,
      label: g.label,
      entries: g.entries.map((e) => ({
        id: e.entry.id,
        kind: e.entry.kind,
        name: e.entry.name,
        publisher: e.entry.publisher,
        licence: e.entry.licence,
        status: statusOf(e, s.installing, servers).text,
        installed: e.installed?.version ?? null,
        updateAvailable: e.installed?.updateAvailable ?? false,
        installable: e.installable,
        access: e.entry.server?.access ?? null,
        inSession:
          e.inSession === null
            ? null
            : {
                loaded: e.inSession.loaded,
                skills: e.inSession.skills.map((k) => ({ name: k.name, enabled: k.enabled })),
                issues: e.inSession.issues,
              },
      })),
    })),
  };
}

/** Runs one `ui/marketplace` request. */
async function handleMarket(app: App, body: unknown): Promise<unknown> {
  const input = asObject(body);
  const market = app.market;
  if (market === null) throw new UiControlError("serverUnavailable", "the app has no core");
  const a = market.debugApi;
  try {
    const action = typeof input["action"] === "string" ? input["action"] : "read";
    switch (action) {
      case "read":
        break;
      case "open":
        await a.showTab("marketplace");
        break;
      case "connectors":
        await a.showTab("connectors");
        break;
      case "reload":
        await a.load();
        break;
      case "filter": {
        const filter = input["filter"];
        if (!MARKET_FILTERS.some((f) => f.id === filter))
          throw new UiControlError("invalidParams", "filter must be all, installed or updates");
        a.setFilter(filter as MarketFilter);
        break;
      }
      case "search":
        a.setSearch(typeof input["text"] === "string" ? input["text"] : "");
        break;
      case "select": {
        const id = input["id"];
        if (id !== null && typeof id !== "string")
          throw new UiControlError("invalidParams", "id must be a string or null");
        a.select(id ?? null);
        break;
      }
      case "install":
        await a.install(text(input, "id"));
        break;
      case "offer":
        await a.offerInstall(text(input, "id"));
        break;
      case "installOffer":
        await a.installShown(text(input, "id"));
        break;
      case "cancelOffer":
        a.cancelOffer(text(input, "id"));
        break;
      case "cancel":
        await a.cancel();
        break;
      case "remove":
        await a.remove(text(input, "id"));
        break;
      case "turnOffPlugin":
      case "turnOnPlugin":
      case "removePlugin":
        await a.changePlugin(action, text(input, "path"));
        break;
      case "addServer":
        await a.addServer(text(input, "id"));
        break;
      default:
        throw new UiControlError(
          "invalidParams",
          `unknown action ${JSON.stringify(input["action"])}`,
        );
    }
  } catch (thrown) {
    if (thrown instanceof UiControlError) throw thrown;
    const error = toCallError(thrown).error;
    throw new UiControlError(error.code, error.message);
  }
  return read(app);
}

/** Registers the route; returns the function that removes it. */
export function registerMarketRoutes(app: App): () => void {
  return app.uiControl.register("marketplace", (body) => handleMarket(app, body));
}
