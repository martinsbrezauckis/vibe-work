/**
 * "Point Vibe at this": the one send path from a terminal selection (the selection API), an element of
 * the session's preview (engine pick, engine) or a section of a page (engine pick checked against the page file).
 * Every one ends as a CONTEXT CHIP in the session's composer: the user decides what to write and when to send, and
 * the chip goes out as a resource block, never as typed text, never as `!cmd` or `/cmd`.
 * Depends on: model.ts (chips), store.ts (page picks), terminal/selection, protocol types, AppContext types.
 * Called by: the Terminal and Browser toolbars, the Project tab, pagesRoutes.ts.
 * Invariants: a point on the New chat screen (its owner id) lands in the New chat composer; `origin` "debug-api" uses the composer's Debug API facade (the chip is marked as the Debug API's, so
 * a send goes out with that origin); the UI origin uses the window's own facade.
 */
import { toCallError } from "../../protocol/client/types";
import type { PickTarget } from "../../protocol/core";
import { NEW_CHAT } from "../../state/newChatKey";
import type { App } from "../shell/AppContext";
import { readTerminalSelection } from "../terminal/selection";
import {
  type Chip,
  chipFolder,
  elementChip,
  sectionChip,
  terminalChip,
  webSelectionChip,
} from "./model";
import type { Pages } from "./store";

/** The folder `sessionId`'s page chips name (see `chipFolder`). */
export function sessionChipFolder(app: App, pages: Pages, sessionId: string): string {
  const cwd = app.core?.store.get().sessions[sessionId]?.session["cwd"];
  return chipFolder(pages.store.get().hubs[sessionId], typeof cwd === "string" ? cwd : null);
}

/** What a point produced. */
export type PointOutcome = { ok: true; chip: Chip } | { ok: false; reason: string };

/** Inserts `chip` into `sessionId`'s composer with the facade of `origin`. */
export function insertChip(
  app: App,
  sessionId: string,
  chip: Chip,
  origin: "ui" | "debug-api" = "ui",
): PointOutcome {
  const facade = origin === "debug-api" ? app.composer.debugApi : app.composer.actions;
  if (facade === null) return { ok: false, reason: "Vibe Work is not connected." };
  // Before the first message the New chat screen's terminal and browser point into its own composer.
  const key = sessionId === app.shell.store.get().newChatOwner ? NEW_CHAT : sessionId;
  facade.insertContext(key, chip);
  return { ok: true, chip };
}

/** The terminal's selection as a chip. */
export function pointTerminal(
  app: App,
  sessionId: string,
  origin: "ui" | "debug-api" = "ui",
): PointOutcome {
  const selection = readTerminalSelection();
  if (selection.trim() === "") return { ok: false, reason: "Select text in the terminal first." };
  return insertChip(app, sessionId, terminalChip(selection), origin);
}

/**
 * An element of the session's preview (the user selects text or clicks it in the preview). With `target` the element
 * is named instead, which is how a Debug API request points: it has no pointer, and the answer is immediate. With
 * `tabId` (a web page of the Browser tab) only the page's selection is read, and the chip says it is a web page's.
 */
export async function pointPreview(
  app: App,
  sessionId: string,
  origin: "ui" | "debug-api" = "ui",
  target?: PickTarget,
  tabId?: string,
): Promise<PointOutcome> {
  if (app.core === null) return { ok: false, reason: "Vibe Work is not connected." };
  try {
    const result = await app.core.run({
      type: "preview",
      request: {
        op: "pick",
        sessionId,
        ...(target === undefined ? {} : { target }),
        ...(tabId === undefined ? {} : { tabId }),
      },
    });
    if (result.type !== "preview" || result.result.type !== "picked") {
      return { ok: false, reason: "Nothing was picked." };
    }
    const picked = result.result.picked;
    const chip = tabId === undefined ? elementChip(picked) : webSelectionChip(picked);
    return insertChip(app, sessionId, chip, origin);
  } catch (thrown) {
    return { ok: false, reason: toCallError(thrown).error.message };
  }
}

/**
 * A section of the shown page. With `section` the section is named instead (as the page file has it), which is how a
 * Debug API request points: it has no pointer, and the answer is immediate.
 */
export async function pointSection(
  app: App,
  pages: Pages,
  sessionId: string,
  origin: "ui" | "debug-api" = "ui",
  section?: string,
): Promise<PointOutcome> {
  const picked = await pages.pickSection(sessionId, section);
  if (picked === null) {
    return { ok: false, reason: pages.store.get().error[sessionId] ?? "Nothing was picked." };
  }
  return insertChip(
    app,
    sessionId,
    sectionChip(sessionChipFolder(app, pages, sessionId), picked),
    origin,
  );
}
