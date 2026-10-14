/**
 * The in-app toolbar above the body: (phone: menu button), brand, palette launcher (phone: icon), "Debug API on" badge
 * (developer builds only; user builds show it in the status bar), theme switch (the icon names the
 * theme a click switches to; the window cross-fades through a view transition, `state/themeSwitch`),
 * and the notification bell/unread badge next to it. The bell also opens empty retained history.
 * Phones also show the Vault destination here because their side rail is hidden.
 * The launcher's label says only what search covers today: sessions, settings and actions.
 * The native window frame supplies its window buttons.
 * Depends on: AppContext, hooks, components/Icon, BrandMark, state/shortcuts (`formatKeys`).
 * Called by: Shell.
 */
import { BrandMark } from "../../components/BrandMark";
import { Badge, IconButton } from "../../components/Controls";
import { Icon } from "../../components/Icon";
import { formatKeys } from "../../state/shortcuts";
import { useApp } from "./AppContext";
import { switchTheme } from "../../state/themeSwitch";
import { TEXT } from "../../vocabulary";
import { useCoreState, useLayout, useShellState } from "./hooks";
import { useStore } from "../../state/createStore";

/** The title bar. */
export function TitleBar() {
  const { shell, toasts } = useApp();
  const unread = useStore(toasts.history, (list) => list.filter((notice) => !notice.read).length);
  const notificationsOpen = useStore(toasts.panel, (state) => state.open);
  const debugOn = useCoreState((s) => s.debugApi.enabled);
  // user builds show "Debug API" only in the status bar; developer builds keep this badge too.
  const devBuild = useCoreState((s) => s.debugApi.reason === "dev-build") || import.meta.env.DEV;
  const theme = useShellState((s) => s.theme);
  const vaultOpen = useShellState((s) => s.settings.open && s.settings.category === "secrets");
  const phone = useLayout().phone;
  return (
    <header className="titlebar">
      {phone && (
        <IconButton
          icon="panel-left"
          aria-label="Open menu"
          title="Open menu"
          onClick={shell.actions.toggleDrawer}
        />
      )}
      <div className="brand">
        <BrandMark />
        <span className="brand-name">Vibe Work</span>
      </div>
      <div className="spacer" />
      {phone ? (
        <IconButton
          icon="search"
          aria-label="Search sessions, messages, files and settings"
          title="Search"
          onClick={() => {
            shell.actions.setOverlay("palette");
          }}
        />
      ) : (
        <button
          type="button"
          className="search"
          // The visible label stays short enough for a 960 px window; the name says all it covers.
          aria-label="Search sessions, messages, files and settings"
          onClick={() => {
            shell.actions.setOverlay("palette");
          }}
        >
          <Icon name="search" />
          <span className="s-label">Search sessions, messages and files</span>
          <kbd>{formatKeys("Mod+K")}</kbd>
        </button>
      )}
      <div className="spacer" />
      {phone && (
        <IconButton
          icon="key-round"
          data-rail="vault-narrow"
          aria-label={TEXT.vault}
          aria-current={vaultOpen ? "true" : undefined}
          title={TEXT.vault}
          onClick={() => {
            shell.actions.openSettings("secrets");
          }}
        />
      )}
      {debugOn && devBuild && (
        <button
          type="button"
          className="dbg"
          aria-label="Debug API on"
          title="An external agent can drive this app over the local Debug API. Click for its setting."
          onClick={() => {
            shell.actions.openSettings("developer", "debug-api");
          }}
        >
          <i />
          <span className="dbg-text">Debug API on</span>
        </button>
      )}
      <IconButton
        id="notifications-trigger"
        icon="bell"
        aria-label="Notifications"
        aria-describedby={unread > 0 ? "notifications-unread" : undefined}
        title="Notifications"
        aria-haspopup="dialog"
        popoverTarget="notifications-panel"
        popoverTargetAction="show"
        aria-expanded={notificationsOpen}
        aria-controls={notificationsOpen ? "notifications-panel" : undefined}
        onClick={() => {
          if (notificationsOpen) toasts.close();
          else toasts.open();
        }}
      >
        {unread > 0 && (
          <Badge id="notifications-unread" aria-label={`${String(unread)} unread`}>
            {unread}
          </Badge>
        )}
      </IconButton>
      <IconButton
        icon={theme === "dark" ? "sun" : "moon"}
        data-theme-switch="true"
        aria-label="Switch theme"
        title={theme === "dark" ? "Switch to the light theme" : "Switch to the dark theme"}
        onClick={() => {
          switchTheme(theme === "dark" ? "light" : "dark", shell.actions.setTheme);
        }}
      />
    </header>
  );
}
