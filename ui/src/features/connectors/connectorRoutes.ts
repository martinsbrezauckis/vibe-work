/**
 * The `ui/connectors` UI-control route. The Rust side lists the route name
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES). Every action calls the same function a click calls, through
 * the Debug API's action set (`app.connectors.debugApi`), so every core call it triggers carries the Debug
 * API origin and every change asks the native confirmation first:
 * - `read` (default): page open, filter, search, groups with each connector's status, the servers, the
 *   sign-in in progress (kind, id, phase, page address, message), prompts, selection, add dialog, note;
 * - `open`, `close`, `filter {filter}`, `search {text}`, `select {id|null}`, `refresh`;
 * - `signIn {alias}`, `stopWaiting`, `turnOff {alias}`, `turnOn {alias}`;
 * - `addServer {url, name?}`, `signInServer {name}`, `signOutServer {name}`, `removeServer {name}`,
 *   `serverOff {name}`, `serverOn {name}`;
 * -: `addKeyServer {url, name?, secretId? | newSecret {name, value}, header?}` and `allowServerKey {name}`:
 *   the Debug API starts them, the native dialog in the window decides (the core never answers that dialog for the
 *   Debug API); `read` lists each server's key (secret name, header, grant), never a value.
 * Depends on: AppContext, uiControl, model/view. Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); a sign-in finishes only in a browser.
 */
import { toCallError } from "../../protocol/client/types";
import type { KeyChoice } from "../../state/connectors";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { FILTERS, type Filter, groups } from "./model/view";

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
  const s = app.connectors?.store.get();
  if (s === undefined) return null;
  return {
    open: s.open,
    filter: s.filter,
    search: s.search,
    selected: s.selected,
    addOpen: s.addOpen,
    note: s.note,
    error: s.error,
    catalog: s.view?.catalog ?? null,
    catalogError: s.view?.catalogError ?? null,
    manageUrl: s.view?.manageUrl ?? null,
    sessionId: s.view?.sessionId ?? null,
    groups: groups(s.view, s.filter, s.search).map((g) => ({
      category: g.id,
      label: g.label,
      connectors: g.tiles.map((t) => ({
        alias: t.alias,
        name: t.meta.name,
        status: t.status,
        statusText: t.statusText,
        reason: t.reason,
        tools: t.connector.tools.map((x) => x.name),
      })),
    })),
    servers: (s.view?.servers ?? []).map((x) => ({
      name: x.name,
      address: x.address,
      signIn: x.signIn,
      status: x.status,
      off: x.off,
      tools: x.tools.map((t) => t.name),
      plugin: x.plugin,
      configured: x.configured,
      key: x.key,
    })),
    signIn: s.signIn,
    prompts: s.prompts.map((p) => ({
      alias: p.id,
      name: p.name,
      url: p.url,
      sessionId: p.sessionId,
    })),
  };
}

/** The key part of `addKeyServer`: `secretId` or `newSecret {name, value}`, and `header`. */
function keyOf(input: Record<string, unknown>): KeyChoice {
  const header = typeof input["header"] === "string" ? input["header"] : null;
  if (typeof input["secretId"] === "string")
    return { secretId: input["secretId"], newSecret: null, header };
  const fresh = input["newSecret"];
  if (typeof fresh === "object" && fresh !== null) {
    const o = fresh as Record<string, unknown>;
    if (typeof o["name"] === "string" && typeof o["value"] === "string")
      return { secretId: null, newSecret: { name: o["name"], value: o["value"] }, header };
  }
  throw new UiControlError("invalidParams", "give secretId or newSecret {name, value}");
}

/** Runs one `ui/connectors` request. */
async function handleConnectors(app: App, body: unknown): Promise<unknown> {
  const input = asObject(body);
  const connectors = app.connectors;
  if (connectors === null) throw new UiControlError("serverUnavailable", "the app has no core");
  const a = connectors.debugApi;
  const name = (key: string) => {
    const alias = text(input, key);
    return {
      alias,
      label:
        connectors.store.get().view?.connectors.find((c) => c.alias === alias)?.displayName ??
        alias,
    };
  };
  try {
    switch (input["action"] ?? "read") {
      case "read":
        break;
      case "open":
        await a.open();
        break;
      case "close":
        a.close();
        break;
      case "filter": {
        const filter = input["filter"];
        if (!FILTERS.some((f) => f.id === filter))
          throw new UiControlError("invalidParams", "filter must be all, connected, signin or off");
        a.setFilter(filter as Filter);
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
      case "refresh":
        await a.refresh();
        break;
      case "signIn":
        await a.signIn(text(input, "alias"));
        break;
      case "stopWaiting":
        await a.stopWaiting();
        break;
      case "turnOff":
      case "turnOn": {
        const n = name("alias");
        await a.turnOff(n.alias, input["action"] === "turnOff", n.label);
        break;
      }
      case "addServer":
        await a.addServer(
          text(input, "url"),
          typeof input["name"] === "string" ? input["name"] : null,
        );
        break;
      case "addKeyServer":
        await a.addKeyServer(
          text(input, "url"),
          typeof input["name"] === "string" ? input["name"] : null,
          keyOf(input),
        );
        break;
      case "allowServerKey":
        await a.allowServerKey(text(input, "name"));
        break;
      case "signInServer":
        await a.signInServer(text(input, "name"));
        break;
      case "signOutServer":
        await a.signOutServer(text(input, "name"));
        break;
      case "removeServer":
        await a.removeServer(text(input, "name"));
        break;
      case "serverOff":
      case "serverOn":
        await a.setServerOff(text(input, "name"), input["action"] === "serverOff");
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
export function registerConnectorRoutes(app: App): () => void {
  return app.uiControl.register("connectors", (body) => handleConnectors(app, body));
}
