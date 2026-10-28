/**
 * The `ui/project` route: lets an external agent read and drive the Project tab the way
 * a user does. `ui/pages` (the previous route name) answers the same for one release (v0.1.x), then goes. The
 * Rust side lists both routes (`crates/vw-host/src/host.rs` PAGES_ROUTES).
 * Actions (body `{action,...}`; the selected session):
 * - `state` (default): the view (hub or page), the hub's pages and flags, the page's steps, sections and comments,
 *   the viewer;
 * - `hub`, `open {path}`, `plan`, `back`, `forward`, `reload`;
 * - `pick {section}`: the section pick; no person clicks for a Debug API request, so it names the section (as the page
 *   file has it) and the answer is immediate, `{picked, ...state}`;
 * - `comment {path, section, text, quote?, send?}`, `send {id}`, `delete {id}`;
 * - `point {source: terminal|preview|page, ...}`: "Point Vibe at this" (a context chip in the composer); `preview`
 *   names its element with `target` (`selector`, or `x` and `y`), `page` names its `section`;
 * - `open {path, anchor}` scrolls to a section or step; `answers`, `answer`, `sendAnswers`, `showAnswers`, `check`,
 *   `export` (handbookRoutes.ts); the state also says `prototype`, `answers`, `answersShown` (the Answers panel shows
 *   every control, not only its one line) and `visual` for the shown page.
 * Depends on: state/uiControl, store.ts, point.ts, model.ts, AppContext types. Called by: register.tsx.
 * Invariants: bodies are validated (`invalidParams`); while a route request is served every core call carries the
 * Debug API origin (`uiControl` lowering; a label with no native confirmation); chips inserted
 * here use the composer's Debug API facade (marked, sent with the Debug API origin); nothing here can run a command.
 */
import { writeClipboard } from "../../state/clipboard";
import type { PickTarget } from "../../protocol/core";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { handbookAction, isHandbookAction } from "./handbookRoutes";
import { commentChip, sectionChip } from "./model";
import { insertChip, pointPreview, pointSection, pointTerminal, sessionChipFolder } from "./point";
import type { Pages } from "./store";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(value: unknown, name: string): string {
  if (typeof value !== "string" || value === "")
    throw new UiControlError("invalidParams", `${name} must be a non-empty string`);
  return value;
}

/** The element a preview point names (no person clicks for a Debug API request); required, never waited for. */
function pickTarget(value: unknown): PickTarget {
  if (typeof value === "object" && value !== null && !Array.isArray(value)) return value;
  throw new UiControlError(
    "invalidParams",
    "target is required: target.selector, or target.x and target.y",
  );
}

/** What the tab shows for `sessionId`. */
function state(app: App, pages: Pages, sessionId: string) {
  const s = pages.store.get();
  const hub = s.hubs[sessionId];
  const page = s.pages[sessionId] ?? null;
  const viewer = hub?.viewer ?? null;
  return {
    sessionId,
    view: viewer?.path != null && s.hubView[sessionId] !== true ? "page" : "hub",
    folder: hub?.settings.folder ?? null,
    index: hub?.hub?.index ?? null,
    manifest: hub?.hub?.manifest ?? null,
    counts:
      hub?.hub == null
        ? null
        : {
            errors: hub.hub.validation.errors,
            warnings: hub.hub.validation.warnings,
            infos: hub.hub.validation.infos,
          },
    hub:
      hub?.hub?.groups.map((g) => ({
        group: g.id,
        pages: g.pages.map((p) => ({
          path: p.path,
          title: p.title,
          kind: p.kind,
          status: p.status,
          lite: p.lite,
          flags: hub.hub?.validation.findings.filter((f) => f.page === p.path).map((f) => f.rule),
        })),
      })) ?? null,
    page:
      page === null
        ? null
        : {
            path: page.path,
            title: page.facts.title,
            steps: page.facts.steps,
            sections: page.facts.sections.map((x) => x.id),
            attached: page.attached,
            detached: page.detached,
          },
    viewer,
    picking: s.picking[sessionId] === true,
    error: s.error[sessionId] ?? null,

    prototype: page?.facts.kind === "prototype",
    answers: s.answers[sessionId] ?? null,
    answersShown: app.shell.store.get().answersShown,
    visual: page === null ? null : (hub?.visual[page.path] ?? null),
  };
}

export { registerPlanRoute } from "./handbookRoutes";

/** Registers `ui/project` and its pre- alias `ui/pages`; returns the function that removes both. */
export function registerPagesRoute(app: App, pages: Pages): () => void {
  const handler = (body: unknown) => route(app, pages, body);
  const removers = [
    app.uiControl.register("project", handler),
    // Alias for one release (v0.1.x): the route's name before the tab became "Project".
    app.uiControl.register("pages", handler),
  ];
  return () => {
    for (const remove of removers) remove();
  };
}

/** One `ui/project` request (see the file header). */
async function route(app: App, pages: Pages, body: unknown): Promise<unknown> {
  const input = asObject(body);
  const sessionId =
    typeof input["sessionId"] === "string" ? input["sessionId"] : app.shell.store.get().selected;
  if (sessionId === null) throw new UiControlError("invalidParams", "no session is shown");
  const action = input["action"] ?? "state";
  switch (action) {
    case "state":
      await pages.load(sessionId);
      return state(app, pages, sessionId);
    case "hub":
      pages.showHub(sessionId);
      await pages.load(sessionId);
      return state(app, pages, sessionId);
    case "open": {
      const anchor = input["anchor"];
      if (typeof anchor === "string" && anchor !== "")
        await pages.openAt(sessionId, text(input["path"], "path"), anchor);
      else await pages.open(sessionId, text(input["path"], "path"));
      return state(app, pages, sessionId);
    }
    case "plan":
      await pages.openPlan(sessionId);
      return state(app, pages, sessionId);
    case "back":
      await pages.back(sessionId);
      return state(app, pages, sessionId);
    case "forward":
      await pages.forward(sessionId);
      return state(app, pages, sessionId);
    case "reload":
      await pages.reload(sessionId);
      return state(app, pages, sessionId);
    case "pointAt": {
      const picked = await pages.menuSection(
        sessionId,
        text(input["section"], "section"),
        typeof input["quote"] === "string" ? input["quote"] : undefined,
      );
      if (picked === null)
        throw new UiControlError(
          "invalidParams",
          pages.store.get().error[sessionId] ?? "nothing was picked",
        );
      return insertChip(
        app,
        sessionId,
        sectionChip(sessionChipFolder(app, pages, sessionId), picked),
        "debug-api",
      );
    }
    case "copy": {
      const copied = await pages.menuCopy(
        sessionId,
        text(input["section"], "section"),
        typeof input["quote"] === "string" ? input["quote"] : undefined,
      );
      if (copied === null)
        throw new UiControlError(
          "invalidParams",
          pages.store.get().error[sessionId] ?? "nothing was copied",
        );
      await writeClipboard(copied);
      return { copied };
    }
    case "pick": {
      // No person clicks for a Debug API request: it names the section and the answer is immediate (never a wait).
      const picked = await pages.pickSection(sessionId, text(input["section"], "section"));
      if (picked === null)
        throw new UiControlError(
          "invalidParams",
          pages.store.get().error[sessionId] ?? "nothing was picked",
        );
      return { picked, ...state(app, pages, sessionId) };
    }
    case "comment": {
      const path = text(input["path"], "path");
      const quote = typeof input["quote"] === "string" ? input["quote"] : undefined;
      const id = await pages.addComment(sessionId, {
        path,
        section: text(input["section"], "section"),
        text: text(input["text"], "text"),
        ...(quote === undefined ? {} : { quote }),
      });
      if (id !== null && input["send"] === true) await sendComment(app, pages, sessionId, id);
      return { commentId: id, ...state(app, pages, sessionId) };
    }
    case "send": {
      await sendComment(app, pages, sessionId, text(input["id"], "id"));
      return state(app, pages, sessionId);
    }
    case "delete": {
      const page = pages.store.get().pages[sessionId];
      if (page == null) throw new UiControlError("invalidParams", "no page is shown");
      await pages.deleteComment(sessionId, page.path, text(input["id"], "id"));
      return state(app, pages, sessionId);
    }
    case "point": {
      const source = input["source"];
      const outcome =
        source === "terminal"
          ? pointTerminal(app, sessionId, "debug-api")
          : source === "preview"
            ? await pointPreview(app, sessionId, "debug-api", pickTarget(input["target"]))
            : source === "page"
              ? await pointSection(
                  app,
                  pages,
                  sessionId,
                  "debug-api",
                  text(input["section"], "section"),
                )
              : null;
      if (outcome === null)
        throw new UiControlError("invalidParams", "source must be terminal, preview or page");
      return outcome;
    }
    default:
      if (isHandbookAction(action)) {
        return {
          result: await handbookAction(app, pages, sessionId, action, input),
          ...state(app, pages, sessionId),
        };
      }
      throw new UiControlError(
        "invalidParams",
        "action must be state, hub, open, plan, back, forward, reload, pick, comment, send, delete, point, answers, answer, sendAnswers, showAnswers, check or export",
      );
  }
}

/** Puts comment `id` of the shown page into the composer (Debug API facade) and marks it sent. */
async function sendComment(app: App, pages: Pages, sessionId: string, id: string): Promise<void> {
  const page = pages.store.get().pages[sessionId];
  const comment =
    page?.attached.find((c) => c.id === id) ?? page?.detached.find((c) => c.id === id);
  if (page == null || comment === undefined)
    throw new UiControlError("invalidParams", "no such comment on the shown page");
  const folder = sessionChipFolder(app, pages, sessionId);
  const outcome = insertChip(app, sessionId, commentChip(folder, comment), "debug-api");
  if (!outcome.ok) throw new UiControlError("invalidParams", outcome.reason);
  await pages.markSent(sessionId, page.path, [id]);
}
