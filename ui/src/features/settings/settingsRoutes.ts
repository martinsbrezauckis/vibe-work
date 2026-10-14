/**
 * The `ui/settings` UI-control route. The Rust side lists the route name
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES). Every action calls the same controller function a click
 * calls, so the route can never do something the Settings screen cannot:
 * - `read` (default): whether the Settings page is open, its category, the search inside Settings and its results,
 *   the rows of the shown category as the user sees them (`rows`: id, label, value from the switch/selector/button
 *   state), the About facts (version, author, contact and source link targets), the personal instruction editor (state,
 *   draft) and the error detail preference;
 * - `open {category?, setting?}` / `close`: the Settings page (a page), at a category and,
 *   with `setting`, at that setting (lit for a moment, as a search hit does);
 * - `category {id}`: show one category; `search {query}`: type into the search inside Settings;
 * - `statusUsage {show?: bool}`: the Appearance switch for the status bar's context readout (returned as `statusUsage`);
 * - `instructions {do: load|type|review|back|discard|save|loadCli, scope?, text?}`;
 * - `errorDetail {mode: plain|details}`;
 * - `cliSharing {share?: bool}`: the CLI session-sharing switch; saves `share` when given, returns the
 *   switch, the CLI's home and both Vibe versions (applies at the next start).
 * Depends on: AppContext, state/instructions, state/errorDetail, state/uiControl, catalog, AboutSettings (constants).
 * Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); commands the actions trigger go out with the Debug
 * API origin (the core controller lowers them while a UI-control request is served; a label; no
 * native confirmation); there is no action here that writes without the review step, exactly like the UI.
 * Text this route puts into an editor (`type`, `loadCli`) stays marked as the Debug API's, so the user's
 * later Save click still writes with the Debug API origin (`state/instructions.ts`).
 */
import { errorDetail, setErrorDetail } from "../../state/errorDetail";
import {
  type DocView,
  type Instructions,
  instructionsFor,
  isDirty,
} from "../../state/instructions";
import { UiControlError } from "../../state/uiControl";
import { ABOUT_AUTHOR, CONTACT_URL, HOMEPAGE_URL, SOURCE_URL } from "./AboutSettings";
import { offeredCategories } from "./availability";
import { SEARCH_INDEX, searchSettings, settingPath } from "./catalog";
import { MISTRAL_LINKS } from "./mistralLinks";
import type { App } from "../shell/AppContext";

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body === "object" && body !== null && !Array.isArray(body))
    return body as Record<string, unknown>;
  if (body === undefined || body === null) return {};
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function scopeOf(value: unknown): "personal" {
  if (value === "personal") return value;
  throw new UiControlError("invalidParams", "scope must be personal");
}

function viewOf(view: DocView) {
  return {
    exists: view.doc?.exists ?? null,
    editable: view.doc?.editable ?? null,
    path: view.doc?.path ?? null,
    note: view.doc?.note ?? null,
    cliAvailable: view.doc?.cliAvailable ?? null,
    docText: view.doc?.text ?? null,
    draft: view.draft,
    dirty: isDirty(view),
    reviewing: view.reviewing,
    busy: view.busy,
    stale: view.stale,
    notice: view.notice,
    error: view.error,
  };
}

/** The value a row shows: a switch's state, a shared selector's text, else null. */
function rowValue(row: Element): string | boolean | null {
  const sw = row.querySelector('[role="switch"]');
  if (sw !== null) return sw.getAttribute("aria-checked") === "true";
  const choice = row.querySelector('[role="combobox"] .ui-selector-text');
  if (choice !== null) return choice.textContent.trim();
  return null;
}

/** The shown category's rows as the user sees them (read from the page, so they match the screen). */
function rows() {
  return [...document.querySelectorAll(".settings-page [data-setting]")].map((row) => {
    const id = row.getAttribute("data-setting") ?? "";
    return {
      id,
      label: SEARCH_INDEX.find((e) => e.id === id)?.label ?? null,
      // A link row's target, as on its button (nothing is opened or fetched).
      href: MISTRAL_LINKS.find((l) => l.id === id)?.url ?? null,
      value: rowValue(row),
      lit: row.getAttribute("data-flash") === "true",
    };
  });
}

function read(app: App, instructions: Instructions | null) {
  const state = instructions?.store.get();
  const view = app.shell.store.get().settings;
  return {
    open: view.open,
    category: view.category,
    categories: offeredCategories(app).map((c) => ({ id: c.id, label: c.label })),
    query: view.query,
    results: searchSettings(view.query).map((r) => ({
      id: r.id,
      category: r.category,
      label: r.label,
      path: settingPath(r),
    })),
    rows: view.open ? rows() : [],
    about: {
      version: app.core === null ? null : (app.core.store.get().appVersion ?? null),
      author: ABOUT_AUTHOR,
      contact: CONTACT_URL,
      source: SOURCE_URL,
      homepage: HOMEPAGE_URL,
    },
    instructions:
      state === undefined
        ? null
        : {
            personal: viewOf(state.personal),
          },
    errorDetail: errorDetail.get(),
    statusUsage: app.shell.store.get().showUsage,
  };
}

async function instructionsAction(instructions: Instructions, input: Record<string, unknown>) {
  const action = input["do"];
  if (input["scope"] !== undefined) scopeOf(input["scope"]);
  const scope = () => scopeOf(input["scope"]);
  switch (action) {
    case "load":
      await instructions.load(scope(), input["discard"] === true);
      break;
    case "type": {
      const text = input["text"];
      if (typeof text !== "string")
        throw new UiControlError("invalidParams", "text must be a string");
      instructions.setDraft(scope(), text, "debug-api");
      break;
    }
    case "review":
      instructions.review(scope());
      break;
    case "back":
      instructions.back(scope());
      break;
    case "discard":
      instructions.discard(scope());
      break;
    case "save":
      await instructions.save(scope());
      break;
    case "loadCli":
      await instructions.loadCli("debug-api");
      break;
    default:
      throw new UiControlError(
        "invalidParams",
        "do must be load, type, review, back, discard, save or loadCli",
      );
  }
}

/** The ids of the categories this window offers. */
function offeredIds(app: App): string[] {
  return offeredCategories(app).map((c) => c.id);
}

/** Registers the route; returns the function that removes it. */
export function registerSettingsRoutes(app: App): () => void {
  return app.uiControl.register("settings", async (body) => {
    const input = asObject(body);
    const instructions = app.core === null ? null : instructionsFor(app.core);
    switch (input["action"] ?? "read") {
      case "read":
        break;
      case "open": {
        const category = input["category"];
        const setting = input["setting"];
        if (
          category !== undefined &&
          (typeof category !== "string" || !offeredIds(app).includes(category))
        )
          throw new UiControlError(
            "invalidParams",
            `category must be one of ${offeredIds(app).join(", ")}`,
          );
        if (
          setting !== undefined &&
          (typeof setting !== "string" ||
            !SEARCH_INDEX.some((e) => e.id === setting && e.opens === undefined))
        )
          throw new UiControlError(
            "invalidParams",
            "setting must be the id of a setting (see read: results, rows)",
          );
        const entry = SEARCH_INDEX.find((e) => e.id === setting);
        app.shell.actions.openSettings(entry?.category ?? category, entry?.id);
        break;
      }
      case "close":
        app.shell.actions.closeSettings();
        break;
      case "category": {
        const id = input["id"];
        if (typeof id !== "string" || !offeredIds(app).includes(id))
          throw new UiControlError(
            "invalidParams",
            `id must be one of ${offeredIds(app).join(", ")}`,
          );
        if (!app.shell.store.get().settings.open) app.shell.actions.openSettings(id);
        else app.shell.actions.setSettingsCategory(id);
        break;
      }
      case "search": {
        const query = input["query"];
        if (typeof query !== "string")
          throw new UiControlError("invalidParams", "query must be a string");
        if (!app.shell.store.get().settings.open) app.shell.actions.openSettings();
        app.shell.actions.setSettingsQuery(query);
        break;
      }
      case "statusUsage": {
        const show = input["show"];
        if (show !== undefined && typeof show !== "boolean")
          throw new UiControlError("invalidParams", "show must be a boolean");
        if (show !== undefined) app.shell.actions.setShowUsage(show);
        break;
      }
      case "errorDetail": {
        const mode = input["mode"];
        if (mode !== "plain" && mode !== "details")
          throw new UiControlError("invalidParams", "mode must be plain or details");
        setErrorDetail(mode);
        break;
      }
      case "cliSharing": {
        if (app.core === null) throw new UiControlError("notFound", "this run has no core");
        const share = input["share"];
        if (share !== undefined && typeof share !== "boolean")
          throw new UiControlError("invalidParams", "share must be a boolean");
        return { ...read(app, instructions), cliSharing: await app.core.client.cliSharing(share) };
      }
      case "instructions":
        if (instructions === null) throw new UiControlError("notFound", "this run has no core");
        await instructionsAction(instructions, input);
        break;
      default:
        throw new UiControlError(
          "invalidParams",
          "action must be read, open, close, category, search, statusUsage, instructions, errorDetail or cliSharing",
        );
    }
    return read(app, instructions);
  });
}
