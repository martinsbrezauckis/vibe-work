/**
 * The icon rail, top to bottom: Home, New chat, Projects, Tools, then the list toggle and Settings at the bottom
 * (Home first, "back to my work"; the chat icon opens a NEW
 * chat, never an old one). Home leaves any page on top (Settings, the Projects overview, Tools) and shows the work
 * exactly as it was (`home.ts`); pressed while already home it shows the list (narrow windows: opens the drawer).
 * New chat opens the new-chat view with its composer focused (`newChat.ts`, the same as Ctrl N and the list's
 * button); no session exists until the first send. The sidebar button hides or shows the list; Settings opens and closes
 * the Settings page, which any other rail item leaves first. In narrow
 * windows (no list column) both list buttons open and close the list drawer instead.
 * Depends on: AppContext, hooks, components/Icon (rail icons at the 20 px size token). Called by: Shell.
 * Vault opens Settings > Project secrets from every width. Projects opens the Projects overview in the content area (every project's handbook, plans and
 * sessions); Tools opens the Connectors page; a dot shows a sign-in a session asked for. The two pages
 * replace each other; Home closes both. Tasks gets its own rail button when the feature is available.
 */
import { Icon } from "../../components/Icon";
import { TEXT } from "../../vocabulary";
import { useConnectors } from "../connectors/hooks";
import { useProjects } from "../projects/hooks";
import { useLibraries } from "../libraries/hooks";
import { useApp } from "./AppContext";
import { awayFromHome, goHome } from "./home";
import { openNewChat } from "./newChat";
import { formatKeys } from "../../state/shortcuts";
import { useLayout, useSessionSummaries, useShellState } from "./hooks";

/** The rail. */
export function Rail() {
  const app = useApp();
  const { shell, connectors, projects, libraries } = app;
  const layout = useLayout();
  const toolsOpen = useConnectors((s) => s.open);
  const projectsOpen = useProjects((s) => s.open);
  const librariesOpen = useLibraries((s) => s.open);
  const asked = useConnectors((s) => s.prompts.length > 0);
  const needsYou = useSessionSummaries().some(
    (s) => s.badge === "input" || (s.unread && s.badge !== "run"),
  );
  const settingsOpen = useShellState((s) => s.settings.open);
  const vaultOpen = useShellState((s) => s.settings.open && s.settings.category === "secrets");
  // Any other rail item leaves the Settings page first (the view underneath is what the item then acts on).
  const leaveSettings = () => {
    if (settingsOpen) shell.actions.closeSettings();
  };
  const workCurrent = !settingsOpen && !toolsOpen && !projectsOpen && !librariesOpen;
  // On the new-chat view New chat is the current item; on a session, Home is.
  const onNewChat = useShellState((s) => s.selected === null);
  return (
    <nav className="rail" aria-label="Sections" data-toast-avoid="rail">
      {/* Home first: back to the work exactly as it was; the list column is its "chats". */}
      <button
        type="button"
        className="rbtn"
        aria-current={workCurrent && !onNewChat ? "true" : undefined}
        aria-label="Home: back to your work"
        title="Home: back to your work"
        data-rail="home"
        onClick={() => {
          if (awayFromHome(app)) goHome(app);
          else if (layout.drawer) shell.actions.toggleDrawer();
          else if (!layout.listVisible) shell.actions.showList();
        }}
      >
        <Icon name="house" size={20} />
        {needsYou && <span className="dot" role="img" aria-label="Something needs you" />}
      </button>
      <button
        type="button"
        className="rbtn"
        aria-current={workCurrent && onNewChat ? "true" : undefined}
        aria-label="New chat"
        title={`New chat (${formatKeys("Mod+N")})`}
        data-rail="new-chat"
        onClick={() => {
          openNewChat(app);
        }}
      >
        <Icon name="message-square-plus" size={20} />
      </button>
      {projects === null ? null : (
        <button
          type="button"
          className="rbtn"
          aria-current={projectsOpen && !settingsOpen ? "true" : undefined}
          aria-label="Projects: overview of every project"
          title="Projects: overview of every project"
          data-rail="projects"
          onClick={() => {
            if (projectsOpen && !settingsOpen) projects.close();
            else {
              leaveSettings();
              connectors?.actions.close();
              void projects.open();
            }
          }}
        >
          <Icon name="folders" size={20} />
        </button>
      )}
      {connectors === null ? null : (
        <button
          type="button"
          className="rbtn"
          aria-current={toolsOpen && !settingsOpen ? "true" : undefined}
          aria-label="Tools: connectors and MCP servers"
          title="Tools: connectors and MCP servers"
          data-rail="tools"
          onClick={() => {
            if (toolsOpen && !settingsOpen) connectors.actions.close();
            else {
              leaveSettings();
              projects?.close();
              void connectors.actions.open();
            }
          }}
        >
          <Icon name="plug" size={20} />
          {asked && <span className="dot" role="img" aria-label="A connector needs a sign-in" />}
        </button>
      )}
      {libraries === null ? null : (
        <button
          type="button"
          className="rbtn"
          aria-current={librariesOpen && !settingsOpen ? "true" : undefined}
          aria-label="Libraries: your documents to ask across"
          title="Libraries: your documents to ask across"
          data-rail="libraries"
          onClick={() => {
            if (librariesOpen && !settingsOpen) libraries.actions.close();
            else void libraries.actions.open();
          }}
        >
          <Icon name="book-open" size={20} />
        </button>
      )}
      <button
        type="button"
        className="rbtn"
        aria-current={vaultOpen ? "true" : undefined}
        aria-label={TEXT.vault}
        title={TEXT.vaultRailTooltip}
        data-rail="vault"
        onClick={() => {
          shell.actions.openSettings("secrets");
        }}
      >
        <Icon name="key-round" size={20} />
      </button>
      <div className="grow" />
      <button
        type="button"
        className="rbtn"
        aria-label="Hide or show the list"
        aria-pressed={layout.listVisible || layout.listDrawerOpen}
        title={`Hide or show the list (${formatKeys("Mod+B")})`}
        data-rail="list"
        disabled={settingsOpen && !layout.drawer}
        onClick={() => {
          if (layout.drawer) shell.actions.toggleDrawer();
          else shell.actions.toggleList(layout.listVisible);
        }}
      >
        <Icon name="panel-left" size={20} />
      </button>
      <button
        type="button"
        className="rbtn"
        aria-current={settingsOpen && !vaultOpen ? "true" : undefined}
        aria-label="Settings"
        title={`Settings (${formatKeys("Mod+,")})`}
        data-rail="settings"
        onClick={() => {
          if (settingsOpen) shell.actions.closeSettings();
          else shell.actions.openSettings();
        }}
      >
        <Icon name="settings" size={20} />
      </button>
    </nav>
  );
}
