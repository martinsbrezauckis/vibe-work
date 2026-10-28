/**
 * The `allowed` action family of the `ui/settings` UI-control route (Settings › Safety's level links and summary,
 * Settings › Allowed without asking). Every action runs the same store function the click runs:
 * - `{do: "read"}` (default): the page as shown (count line, groups, sub-groups, rows shown, bulk links, the open
 *   confirmation, status and problem lines), the Safety summary and the level rows;
 * - `open {sessionId?}` (the session grants list's link), `review` (Safety's Review link), `level {row}` (a level
 *   row's link: access, chrome, askFirst, closeTabs);
 * - `find {query}` (also matches a project's folder name), `clear` (Clear the search), `toggle {key}` (a group or
 *   sub-group header), `more {key}` (Show 20 more);
 * - `revoke {key}` (a row's Revoke), `revokeAll {key}` (opens the confirmation), `cancel`, `confirm`;
 * - `openSession {sessionId}`, `openSecrets` (Open Project secrets).
 * Depends on: state/uiControl, AppContext types, allowed/store, open, levels, view. Called by: settingsRoutes.
 * Invariants: a refused command is reported as the route's error with the core's code and words (the vault's
 * revokes are the user's own: under UI control the core refuses them `vaultUserOnly`, and the page shows the same
 * words); nothing here works around a refusal.
 */
import { UiControlError } from "../../../state/uiControl";
import type { App } from "../../shell/AppContext";
import { allowedEntries } from "./entries";
import { type LevelRowId, openLevel } from "./levels";
import { openAllowed } from "./open";
import { allowedFor, type Outcome } from "./store";
import { type LeafView, moreLabel, summaryView } from "./view";

function text(input: Record<string, unknown>, name: string): string {
  const value = input[name];
  if (typeof value !== "string")
    throw new UiControlError("invalidParams", `${name} must be a string`);
  return value;
}

function leafJson(l: LeafView, busy: Readonly<Record<string, true>>) {
  return {
    key: l.key,
    title: l.title,
    open: l.open,
    count: l.entries.length,
    bulk: l.bulk,
    more: l.notShown > 0 ? moreLabel(l.notShown) : null,
    sessionId: l.sessionId,
    rows: l.rows.map((e) => ({
      key: e.key,
      kind: e.kind,
      what: e.what.map((p) => p.text).join(""),
      detail: e.detail,
      project: e.project,
      busy: busy[e.key] === true,
    })),
  };
}

/** What the page and the Safety summary show now. */
function allowedRead(app: App) {
  const allowed = allowedFor(app);
  const s = allowed.state.get();
  const view = allowed.view();
  const sources = allowed.sources();
  return {
    state: view.state,
    filtering: view.filtering,
    countLine: view.countLine,
    query: s.ui.query,
    groups: view.groups.map((g) => ({
      id: g.id,
      title: g.title,
      count: g.count,
      open: g.open,
      problem: g.problem,
      locked: g.locked,
      note: g.note,
      status: s.status[g.id] ?? null,
      refused: s.problem[g.id] ?? null,
      leaf: g.leaf === null ? null : leafJson(g.leaf, s.busy),
      subgroups: g.subgroups.map((l) => leafJson(l, s.busy)),
    })),
    dialog: s.dialog,
    summary: summaryView(sources, allowedEntries(sources, Date.now())),
  };
}

const LEVELS: readonly LevelRowId[] = ["access", "chrome", "askFirst", "closeTabs"];

/** A refused action becomes the route's error, in the core's words. */
function report(outcome: Outcome): void {
  if (!outcome.ok) {
    throw new UiControlError(
      outcome.code,
      outcome.serverCode === null ? outcome.message : `${outcome.message} (${outcome.serverCode})`,
    );
  }
}

/** Runs one `allowed` action; returns the page as it then is. */
export async function allowedAction(app: App, input: Record<string, unknown>) {
  const allowed = allowedFor(app);
  switch (input["do"] ?? "read") {
    case "read":
      break;
    case "open": {
      const sessionId = input["sessionId"];
      if (sessionId !== undefined && typeof sessionId !== "string")
        throw new UiControlError("invalidParams", "sessionId must be a string");
      openAllowed(app, sessionId === undefined ? {} : { sessionId });
      await allowed.load();
      break;
    }
    case "review":
      openAllowed(app);
      await allowed.load();
      break;
    case "level": {
      const row = input["row"];
      if (typeof row !== "string" || !LEVELS.includes(row as LevelRowId))
        throw new UiControlError("invalidParams", `row must be one of ${LEVELS.join(", ")}`);
      openLevel(app, row as LevelRowId);
      break;
    }
    case "find":
      allowed.setQuery(text(input, "query"));
      break;
    case "clear":
      allowed.clearSearch();
      break;
    case "toggle":
      allowed.toggle(text(input, "key"));
      break;
    case "more":
      allowed.showMore(text(input, "key"));
      break;
    case "revoke":
      report(await allowed.revoke(text(input, "key")));
      break;
    case "revokeAll":
      report(allowed.askRevokeAll(text(input, "key")));
      break;
    case "cancel":
      allowed.cancelRevokeAll();
      break;
    case "confirm":
      report(await allowed.confirmRevokeAll());
      break;
    case "openSession":
      await allowed.openSession(text(input, "sessionId"));
      break;
    case "openSecrets":
      allowed.openProjectSecrets();
      break;
    default:
      throw new UiControlError(
        "invalidParams",
        "do must be read, open, review, level, find, clear, toggle, more, revoke, revokeAll, cancel, confirm, openSession or openSecrets",
      );
  }
  return allowedRead(app);
}
