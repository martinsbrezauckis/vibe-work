/**
 * UI entry point: builds the app (client, stores, registries), starts the core connection and
 * mounts the shell.
 * Depends on: protocol/client (selectClient), state/*, features/shell. Called by: index.html
 * (module script).
 * Invariants: the window always renders: a missing or failed core is shown as state, never as a
 * blank screen; one `App` per page load.
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { AppProvider, type App } from "./features/shell/AppContext";
import { createApp } from "./features/shell/createApp";
import { registerChangesTab } from "./features/changes/register";
import { registerGitBrowser } from "./features/git-browser/register";
import { registerPlaceholderTabs } from "./features/shell/PlaceholderTabs";
import { registerFilesTab } from "./features/files/register";
import { registerPagesTab } from "./features/pages/register";
import { registerPreviewTab } from "./features/preview/register";
import { registerProjects } from "./features/projects/register";
import { Shell } from "./features/shell/Shell";
import { registerTerminalTab } from "./features/terminal/register";
import { selectClient } from "./protocol/client/select";
import "./styles/tokens.css";
import "./styles/shell.css";
import "./styles/content.css";
import "./styles/panel.css";
import "./styles/transcript.css";
import "./styles/transcript-cards.css";
import "./styles/responsive.css";
import "./styles/overlays.css";
import "./styles/sessions.css";
import "./styles/composer.css";
import "./styles/loops.css";
import "./styles/documents.css";
import "./styles/voice.css";
import "./styles/terminal.css";
import "./styles/setup.css";
import "./styles/settings.css";
import "./styles/changes.css";
import "./styles/line-comments.css";
import "./styles/preview.css";
import "./styles/pages.css";
import "./styles/pages-h1.css";
import "./styles/files.css";
import "./styles/projects.css";
import "./styles/connectors.css";
import "./styles/market.css";
import "./styles/libraries.css";
import "./styles/worktrees.css";
import "./styles/git.css";
import "./styles/git-browser.css";
import "./styles/explorer.css";
import "./styles/updates.css";
import "./styles/settings-page.css";
import "./styles/mistral-keys.css";
import "./styles/shared-controls.css";
import "./styles/search.css";
// the shared motion and responses load last so they win over older per-feature rules.
import "./styles/feel.css";
import "./styles/editor-workspace.css";

const container = document.getElementById("root");
if (container === null) {
  throw new Error("index.html is missing the #root element");
}

const app: App = createApp({ client: selectClient() });
registerPlaceholderTabs(app.panelTabs);
registerTerminalTab(app);
registerChangesTab(app);
registerGitBrowser(app);
registerPreviewTab(app);
registerPagesTab(app);
registerFilesTab(app);
registerProjects(app);
void app.core?.start();
void app.newChat?.start();

createRoot(container).render(
  <StrictMode>
    <AppProvider app={app}>
      <Shell />
    </AppProvider>
  </StrictMode>,
);
