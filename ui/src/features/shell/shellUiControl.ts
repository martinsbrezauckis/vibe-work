/**
 * The `ui/*` routes the shell owns: `panel`, `state`, `theme`, `list`, `session`, `rail`. The
 * Rust side lists the same names (`crates/vw-host/src/host.rs` SHELL_ROUTES); end-to-end tests post each one. Handlers act through the same actions a click uses, so a route
 * can never do something the UI cannot.
 * Depends on: AppContext types, state/*. Called by: useShellWiring.
 * Invariants: every body is validated; a bad body is an `invalidParams` error, not a crash; `state`
 * is read-only and returns plain JSON (no tokens, no paths beyond the session folder names the
 * list already shows).
 */
import { badgeOf, summarize } from "../../state/sessionSummary";
import { computeLayout } from "../../state/panelLayout";
import { TAB_KINDS, type TabKind, panelOf } from "../../state/shellState";
import { filterCommands } from "../../state/commands";
import { UiControlError } from "../../state/uiControl";
import { waitingSessions } from "../ask/waiting";
import type { App } from "./AppContext";
import { awayFromHome, goHome, homeTarget } from "./home";
import { openNewChat } from "./newChat";

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body === "object" && body !== null && !Array.isArray(body))
    return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function tabOf(value: unknown): TabKind {
  // "pages" is the Project tab's name before: accepted as an alias for one release (v0.1.x), then
  // removed. Answers always name the tab "project".
  const tab = TAB_KINDS.find((k) => k === (value === "pages" ? "project" : value));
  if (tab === undefined)
    throw new UiControlError("invalidParams", `tab must be one of ${TAB_KINDS.join(", ")}`);
  return tab;
}

/** Registers the shell routes on `app.uiControl`; returns the function that removes them. */
export function registerShellRoutes(app: App): () => void {
  const { shell, core } = app;
  const layout = () => computeLayout(shell.store.get(), window.innerWidth);
  const panelState = () => {
    const panel = panelOf(shell.store.get());
    const l = layout();
    return {
      open: l.panelOpen,
      tabs: panel.tabs,
      active: panel.open ? panel.active : null,
      width: l.panelWidth,
      maximized: l.maximized,
    };
  };
  const removers = [
    app.uiControl.register("panel", (body) => {
      const { action, tab } = asObject(body);
      const a = shell.actions;
      if (action === "close") a.closePanel();
      else if (action === "open") a.openTab(tabOf(tab));
      else if (action === "toggle") a.toggleTab(tabOf(tab));
      else throw new UiControlError("invalidParams", "action must be open, close or toggle");
      return panelState();
    }),
    app.uiControl.register("theme", (body) => {
      const { theme } = asObject(body);
      if (theme === "toggle") shell.actions.toggleTheme();
      else if (theme === "dark" || theme === "light") shell.actions.setTheme(theme);
      else throw new UiControlError("invalidParams", "theme must be dark, light or toggle");
      return { theme: shell.store.get().theme };
    }),
    app.uiControl.register("list", (body) => {
      const { action } = asObject(body);
      const drawer = layout().drawer;
      if (action === "show") {
        if (drawer) shell.actions.setDrawer(true);
        else shell.actions.showList();
      } else if (action === "hide") {
        if (drawer) shell.actions.setDrawer(false);
        else shell.actions.toggleList(true);
      } else if (action === "toggle") {
        if (drawer) shell.actions.toggleDrawer();
        else shell.actions.toggleList(layout().listVisible);
      } else throw new UiControlError("invalidParams", "action must be show, hide or toggle");
      const after = layout();
      return { visible: after.listVisible || after.listDrawerOpen };
    }),
    // the rail as a click sees it. read: the items in order, which is current, where Home goes; home,
    // newChat and the page items act like their buttons (home = back to the work exactly as it was, see home.ts;
    // newChat = the new-chat view, composer focused, no session until the first send, see newChat.ts).
    app.uiControl.register("rail", (body) => {
      const input = body === undefined || body === null ? {} : asObject(body);
      const action = input["action"] ?? "read";
      if (action === "home") goHome(app);
      else if (action === "newChat") openNewChat(app);
      else if (action === "projects") {
        shell.actions.closeSettings();
        app.connectors?.actions.close();
        void app.projects?.open();
      } else if (action === "tools") {
        shell.actions.closeSettings();
        app.projects?.close();
        void app.connectors?.actions.open();
      } else if (action === "libraries") void app.libraries?.actions.open();
      else if (action === "settings") shell.actions.openSettings();
      else if (action !== "read")
        throw new UiControlError(
          "invalidParams",
          "action must be read, home, newChat, projects, tools, libraries or settings",
        );
      const items = [...document.querySelectorAll<HTMLElement>(".rail [data-rail]")].map((el) => ({
        id: el.dataset["rail"] ?? "",
        label: el.getAttribute("aria-label"),
        current: el.getAttribute("aria-current") === "true",
      }));
      // Read from the stores, not the DOM: the page this call opened or left may not have re-rendered yet.
      const view = shell.store.get();
      const current = view.settings.open
        ? "settings"
        : (app.projects?.store.get().open ?? false)
          ? "projects"
          : (app.connectors?.store.get().open ?? false)
            ? "tools"
            : (app.libraries?.store.get().open ?? false)
              ? "libraries"
              : view.selected === null
                ? "new-chat"
                : "home";
      return {
        items: items.map((i) => ({ id: i.id, label: i.label })),
        current,
        away: awayFromHome(app),
        home: homeTarget(app),
        // The new-chat view's folder chip (who chose it: picker, debug-api or session).
        newChat: { folder: view.newFolder, folderBy: view.newFolderBy },
      };
    }),
    app.uiControl.register("session", (body) => {
      const { sessionId } = asObject(body);
      if (sessionId !== null && typeof sessionId !== "string")
        throw new UiControlError("invalidParams", "sessionId must be a string or null");
      shell.actions.selectSession(sessionId);
      return { selected: shell.store.get().selected };
    }),
    app.uiControl.register(
      "state",
      () => {
        const s = shell.store.get();
        const stuck = core?.store.get().stuck ?? {};
        const waiting = core === null ? new Set<string>() : waitingSessions(core.store.get());
        const sessions = Object.values(core?.store.get().sessions ?? {}).map((entry) => ({
          ...summarize(entry, stuck, waiting),
          badge: badgeOf(entry, stuck, waiting),
        }));
        return {
          theme: s.theme,
          list: { visible: layout().listVisible || layout().listDrawerOpen },
          selected: s.selected,
          panel: panelState(),
          server: core?.store.get().server.phase ?? "unavailable",
          debugApi: core?.store.get().debugApi.enabled ?? false,
          // what covers the content (settings, palette,...) and the toasts on screen.
          overlay: s.overlay,
          toasts: app.toasts.store.get().map(({ id, kind, text }) => ({ id, kind, text })),
          // The version Settings > About shows (the core's `status`; `null` before it arrived).
          appVersion: core?.store.get().appVersion ?? null,
          sessions: sessions.map(({ id, title, badge, project, unread, pinned }) => ({
            id,
            title,
            badge,
            project,
            unread,
            pinned,
          })),
        };
      },
      () => true,
    ),
    // the command palette and the toasts, like the keyboard and a click use them.
    app.uiControl.register("palette", (body) => {
      const { action, query, id } = asObject(body ?? {});
      const listed = () =>
        filterCommands(app.commands.list(), typeof query === "string" ? query : "").map(
          ({ id: itemId, label, section }) => ({ id: itemId, label, section }),
        );
      switch (action ?? "list") {
        case "open":
          shell.actions.setOverlay("palette");
          return { open: true, items: listed() };
        case "close":
          shell.actions.setOverlay(null);
          return { open: false };
        case "list":
          return { open: shell.store.get().overlay === "palette", items: listed() };
        case "run": {
          const item = app.commands.list().find((c) => c.id === id);
          if (item === undefined) throw new UiControlError("notFound", "no command with that id");
          shell.actions.setOverlay(null);
          item.run();
          return { ran: item.id };
        }
        default:
          throw new UiControlError("invalidParams", "action must be open, close, list or run");
      }
    }),
    app.uiControl.register("toasts", (body) => {
      const { action, id } = asObject(body ?? {});
      if (action === "dismiss") {
        if (typeof id !== "number")
          throw new UiControlError("invalidParams", "id must be a number");
        app.toasts.dismiss(id);
      } else if (action !== undefined && action !== "read") {
        throw new UiControlError("invalidParams", "action must be read or dismiss");
      }
      return app.toasts.store
        .get()
        .map(({ id: toastId, kind, text }) => ({ id: toastId, kind, text }));
    }),
  ];
  return () => {
    for (const remove of removers) remove();
  };
}
