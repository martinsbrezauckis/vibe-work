/**
 * The `ui/skills` UI-control route: the Tools page's Skills tab, every action its controls have. The Rust side lists
 * the route name (`crates/vw-host/src/host.rs` SHELL_ROUTES). Every action calls the same function a click calls,
 * through the Debug API's action set (`app.skills.debugApi`), so every core call carries the Debug API origin:
 * - `state` (default): whether the tab shows, loading, the list error, note, search, each row as drawn (name,
 *   group, version, latest, update, skillId, enabled, locked, expanded, its error, when its skill file last
 *   changed as `modifiedAt` in milliseconds since the epoch or null) and the install offers
 *   (`offers`: each open offer's phase, id, heading, fact lines, confirm words, notice and its kind changed/ranOut;
 *   read failures; the focus request);
 * - `open` (shows the Skills tab), `reload`, `search {text}`, `disclose {name}`;
 * - `install {name}` = a catalog row's Install…, `update {name}` = an installed row's Update to vN…: both show the
 *   offer; `confirm {name}` = the offer's Install vN / Update to vN (sends that offer's id; a stale offer shows the
 *   fresh one with its notice and is never sent again by itself); `cancel {name}` = Cancel or Escape;
 * - `enable {name, enabled}` = the row's switch.
 * Remove keeps its confirmation dialog in the window (`ui/click`); vault actions are not here.
 * Depends on: AppContext, uiControl, the skills controller and offer model. Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); `state` only reads.
 */
import { toCallError } from "../../protocol/client/types";
import { offersReport } from "../../state/installOffers";
import { isStateRead } from "../../state/routeInput";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";

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

function state(app: App): unknown {
  const skills = app.skills;
  if (skills === null) return null;
  const s = skills.store.get();
  return {
    shown: app.market?.store.get().tab === "skills" && app.connectors?.store.get().open === true,
    loading: s.loading,
    error: s.error,
    note: s.note,
    search: s.search,
    busy: s.view?.busy ?? false,
    authenticated: s.view?.authenticated ?? null,
    rows: (s.view?.skills ?? []).map((r) => ({
      name: r.name,
      group: r.installed ? "installed" : "catalog",
      version: r.installedVersion ?? r.latestVersion,
      latestVersion: r.latestVersion,
      updateAvailable: r.updateAvailable,
      skillId: r.skillId,
      enabled: r.enabled,
      locked: r.locked,
      expanded: s.expanded.includes(r.name),
      error: s.errors[r.name] ?? null,
      modifiedAt: r.modifiedAt,
    })),
    offers: offersReport(skills.offers.get()),
  };
}

/** The row `name`, refused when the list does not show it. */
function row(app: App, name: string) {
  const found = app.skills?.store.get().view?.skills.find((r) => r.name === name);
  if (found === undefined) throw new UiControlError("notFound", `no skill named ${name} is listed`);
  return found;
}

/** Runs one `ui/skills` request and answers the state after it. */
async function handleSkills(app: App, body: unknown): Promise<unknown> {
  const input = asObject(body);
  const skills = app.skills;
  if (skills === null) throw new UiControlError("serverUnavailable", "the app has no core");
  const a = skills.debugApi;
  try {
    const action = typeof input["action"] === "string" ? input["action"] : "state";
    switch (action) {
      case "state":
        break;
      case "open":
        await (app.market?.debugApi.showTab("skills") ?? Promise.resolve());
        break;
      case "reload":
        await a.load();
        break;
      case "search":
        a.setSearch(typeof input["text"] === "string" ? input["text"] : "");
        break;
      case "disclose":
        await a.disclose(row(app, text(input, "name")).name);
        break;
      case "install": {
        const r = row(app, text(input, "name"));
        if (r.installed || r.skillId === null)
          throw new UiControlError(
            "invalidParams",
            `${r.name} has no Install… (not a catalog row)`,
          );
        await a.offerInstall(r.name);
        break;
      }
      case "update": {
        const r = row(app, text(input, "name"));
        if (!r.installed || !r.updateAvailable)
          throw new UiControlError("invalidParams", `${r.name} has no Update to vN…`);
        await a.offerUpdate(r.name);
        break;
      }
      case "confirm": {
        const name = text(input, "name");
        if (skills.offers.get().open[name]?.phase !== "shown")
          throw new UiControlError("invalidParams", `no offer is shown for ${name}`);
        await a.confirmOffer(name);
        break;
      }
      case "cancel":
        a.cancelOffer(text(input, "name"));
        break;
      case "enable": {
        const r = row(app, text(input, "name"));
        if (typeof input["enabled"] !== "boolean")
          throw new UiControlError("invalidParams", "enabled must be true or false");
        await a.change({ op: "setEnabled", name: r.name, enabled: input["enabled"] }, r.name);
        break;
      }
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
  return state(app);
}

/** Registers the route; returns the function that removes it. */
export function registerSkillsRoutes(app: App): () => void {
  return app.uiControl.register("skills", (body) => handleSkills(app, body), isStateRead);
}
