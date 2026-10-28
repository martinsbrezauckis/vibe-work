/**
 * The session grants list as `ui/transcript` route actions, so the Debug API reaches every control
 * of the header chip the way a click does:
 * - `grants {sessionId?}`: the chip and list as drawn (count, open, rows with their words and detail, status);
 * - `grantsOpen {sessionId?, open?}`: the chip's click (toggles without `open`);
 * - `revokeGrant {sessionId?, id}`: a row's Revoke (the core's `askGrants remove`);
 * - `grantsAllowed {sessionId?}`: the list's "See everything allowed without asking".
 * Depends on: grantsModel, model.ts, the App's stores. Called by: transcriptRoutes.
 * Invariant: the same store actions as the chip; an unknown grant is `notFound`.
 */
import { UiControlError } from "../../state/uiControl";
import { openAllowed } from "../settings/allowed/open";
import type { App } from "../shell/AppContext";
import { grantsKey } from "./GrantsChip";
import { chipText, childIds, childName, grantDetail, grantWords, grantsOf } from "./grantsModel";
import { coversText } from "./model";

/** The chip and list of `sessionId` as drawn. */
function grantsState(app: App, sessionId: string): Record<string, unknown> {
  const core = app.core?.store.get();
  const children = core?.sessions[sessionId]?.state?.["childSessions"];
  const grants = grantsOf(core?.askGrants.grants ?? [], sessionId, childIds(children));
  const ui = app.transcript.store.get();
  return {
    sessionId,
    shown: grants.length > 0,
    chip: grants.length > 0 ? chipText(grants.length) : null,
    open: ui.open[grantsKey(sessionId)] === true,
    status: ui.grantStatus[sessionId] ?? "",
    busy: ui.grantBusy[sessionId] ?? null,
    rows: grants.map((g) => ({
      id: g.id,
      session: g.session,
      text: coversText(grantWords(g)),
      detail: grantDetail(
        g,
        g.session === sessionId ? null : (childName(children, g.session) ?? "subagent"),
      ),
      uses: g.uses,
      lastUsedAt: g.lastUsedAt,
    })),
  };
}

/** Runs one grants action for `sessionId`; `null` when `action` is not one of them. */
export async function grantsAction(
  app: App,
  action: string,
  sessionId: string,
  input: Record<string, unknown>,
): Promise<Record<string, unknown> | null> {
  switch (action) {
    case "grants":
      return grantsState(app, sessionId);
    case "grantsOpen": {
      const open = input["open"];
      const now = app.transcript.store.get().open[grantsKey(sessionId)] === true;
      app.transcript.toggle(grantsKey(sessionId), typeof open === "boolean" ? open : !now);
      return grantsState(app, sessionId);
    }
    case "revokeGrant": {
      const id = input["id"];
      if (typeof id !== "string" || id === "")
        throw new UiControlError("invalidParams", "id must be a non-empty string");
      const listed = grantsState(app, sessionId)["rows"] as { id: string }[];
      if (!listed.some((row) => row.id === id))
        throw new UiControlError("notFound", `no grant ${id} in this session's list`);
      const ok = await app.transcript.revokeGrant(sessionId, id);
      return { ok, ...grantsState(app, sessionId) };
    }
    case "grantsAllowed":
      openAllowed(app, { sessionId });
      return { opened: "allowed" };
    default:
      return null;
  }
}
