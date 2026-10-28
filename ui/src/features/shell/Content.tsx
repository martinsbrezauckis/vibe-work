/**
 * The content area: the session header with the side panel buttons, and the view below it: the
 * selected session's transcript with its composer under it (loaded lazily), or
 * the new-chat view (the folder chip and a real composer; the session starts on the first send); the
 * Projects overview or the Tools page replace the view while open. Tools
 * stays available over key/vault and Vibe setup; a server's vault needs belong in its own detail.
 * When Vibe is not running the reason is shown in plain words. The worktree feature adds the header's copy chip, the startup
 * list of copies from deleted sessions and the end-of-session dialog.
 * Both chat views show KeyNotice whenever the Mistral key cannot be used, even after setup is skipped.
 * Depends on: AppContext, hooks, components/Icon, state/shellState, the transcript, composer, worktree, projects
 * and connectors features.
 * Called by: Shell.
 */
import { FoldChatButton } from "./WindowStrips";
import { TEXT } from "../../vocabulary";
import type { ReactNode } from "react";

import { BrandMark } from "../../components/BrandMark";
import { Icon } from "../../components/Icon";
import { useStore } from "../../state/createStore";
import { TAB_KINDS, TAB_META, type TabKind, panelOf } from "../../state/shellState";
import { LazyComposer } from "../composer/LazyComposer";
import { NewChatPanel } from "../sessions/NewChatPanel";
import { NEW_CHAT } from "../../state/newChatKey";
import { covers } from "../../state/keyStore";
import { SetupScreen } from "../setup/SetupScreen";
import { useVibeUi } from "../setup/VibeInstallStep";
import { vibeCovers } from "../../state/vibeInstall";
import { useKeyUi } from "../setup/useKeys";
import { SessionHeldNote, useHeldBy } from "../sessions/SessionHeldNote";
import { TemporaryChip } from "../sessions/TemporaryChip";
import { useFindKeys } from "../search/useFindKeys";
import { Transcript } from "../transcript/Transcript";
import { ConnectorsPage } from "../connectors/ConnectorsPage";
import { SkillsPage } from "../skills/SkillsPage";
import { MarketplacePage } from "../marketplace/MarketplacePage";
import { useMarket } from "../marketplace/hooks";
import { LibrariesPage } from "../libraries/LibrariesPage";
import { useLibraries } from "../libraries/hooks";
import { EndDialog } from "../worktrees/EndDialog";
import { OrphanList } from "../worktrees/OrphanList";
import { IncludeNote } from "../worktrees/IncludeNote";
import { WorktreeChip } from "../worktrees/WorktreeChip";
import { GrantsChip } from "../ask/GrantsChip";
import { PrChip } from "../git/PrChip";
import { useSessionGit } from "../git/useSessionGit";
import { useConnectors } from "../connectors/hooks";
import { ProjectsPage } from "../projects/ProjectsPage";
import { SettingsPage } from "../settings/SettingsPage";
import { useProjects } from "../projects/hooks";
import { useApp } from "./AppContext";
import { KeyNotice } from "./KeyNotice";
import { SendToOffer } from "./SendToOffer";
import { formatKeys } from "../../state/shortcuts";
import { CoveredByPage } from "./coverContext";
import { TAB_ICON } from "./tabIcons";
import { useCoreState, useLayout, useSessionSummaries, useShellState } from "./hooks";

function ServerNote() {
  const server = useCoreState((s) => s.server);
  if (server.phase === "running" || server.phase === "connecting") return null;
  const text =
    server.phase === "unavailable" || server.phase === "dead"
      ? server.reason
      : server.phase === "restarting"
        ? "Vibe stopped and is restarting."
        : server.phase === "starting"
          ? "Vibe is starting."
          : server.phase === "notInstalled"
            ? "Vibe is not installed yet: install it on the setup screen."
            : "Vibe has stopped.";
  return (
    <p className="server-note" role="status">
      <span className="px warn" aria-hidden="true" />
      <span>{text}</span>
    </p>
  );
}

function Header({ title, sessionId }: { title: string; sessionId: string }) {
  const { shell, panelTabs } = useApp();
  const panel = useShellState(panelOf);
  const shown = useLayout().panelOpen;
  const hidden = useStore(panelTabs.hidden, (h) => h);
  const gitStatus = useSessionGit(sessionId);
  return (
    <div className="shead">
      <FoldChatButton />
      <h1>{title}</h1>
      {/* The header's second line: the location chip and the session grants chip, one family. */}
      <div className="shead-line">
        <WorktreeChip sessionId={sessionId} status={gitStatus} />
        <GrantsChip sessionId={sessionId} />
      </div>
      <IncludeNote sessionId={sessionId} />
      <TemporaryChip sessionId={sessionId} />
      <PrChip sessionId={sessionId} />
      {/* toasts keep these buttons clear (Changes, Files,...): a stack moved off the composer to the top
          right covered Changes. */}
      <div className="acts" role="group" aria-label="Side panel" data-toast-avoid="header">
        {TAB_KINDS.filter((kind) => !hidden.includes(kind)).map((kind) => {
          // Shown, not only open: a panel the layout folds (a list opened by hand) is opened by its tab, never closed.
          const active = shown && panel.active === kind;
          const label = `${active ? "Close" : "Open"} ${TAB_META[kind].label} tab${kind === "terminal" && !active ? ` (${formatKeys("Mod+`")})` : ""}`;
          return (
            <button
              data-testid="shell.content-toggle-tab"
              data-item-id={kind}
              key={kind}
              type="button"
              className="pbtn"
              aria-pressed={active}
              title={label}
              aria-label={label}
              onClick={() => {
                if (active) shell.actions.toggleTab(kind);
                else shell.actions.openTab(kind);
              }}
            >
              <Icon name={TAB_ICON[kind]} />
              <span className="lbltxt">{TAB_META[kind].label}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** The New chat screen works before a session exists: Browser, Files and Terminal (no Project, Plan, Changes). */
const NEW_CHAT_TABS: readonly TabKind[] = ["browser", "git", "terminal"];

/** The New chat header's side panel toggles (as a session's, for the tabs that work before the chat). */
function NewChatToggles() {
  const { shell } = useApp();
  const panel = useShellState(panelOf);
  const shown = useLayout().panelOpen;
  return (
    <div className="acts" role="group" aria-label="Side panel" data-toast-avoid="header">
      {NEW_CHAT_TABS.map((kind) => {
        // Shown, not only open: a panel the layout folds (a list opened by hand) is opened by its tab, never closed.
        const active = shown && panel.active === kind;
        const label = `${active ? "Close" : "Open"} ${TAB_META[kind].label} tab${kind === "terminal" && !active ? ` (${formatKeys("Mod+`")})` : ""}`;
        return (
          <button
            data-testid={`shell.welcome-open-${kind}-tab`}
            data-item-id={kind}
            key={kind}
            type="button"
            className="pbtn"
            aria-pressed={active}
            title={label}
            aria-label={label}
            onClick={() => {
              if (active) shell.actions.toggleTab(kind);
              else shell.actions.openTab(kind);
            }}
          >
            <Icon name={TAB_ICON[kind]} />
            <span className="lbltxt">{TAB_META[kind].label}</span>
          </button>
        );
      })}
    </div>
  );
}

/** Makes the composer inert while another process holds the session; layout unchanged. */
function HeldWrap({ sessionId, children }: { sessionId: string; children: ReactNode }) {
  const held = useHeldBy(sessionId) !== null;
  return (
    <div className="held-wrap" inert={held} data-held={held ? "true" : undefined}>
      {children}
    </div>
  );
}

/** The content area. */
export function Content() {
  const selected = useShellState((s) => s.selected);
  const session = useSessionSummaries().find((s) => s.id === selected);
  // The Mistral key setup covers the work view while no working key exists.
  // so does the Vibe install until Vibe is installed or an existing install is chosen.
  const keyCovers = useKeyUi(covers);
  const vibeCovering = useVibeUi(vibeCovers);
  const covered = keyCovers || vibeCovering;
  // Settings, Projects and Tools cover the work view while it stays mounted (inert and hidden). Home, Esc or any rail item
  // returns to it exactly as it was: the same session, scroll, draft and panel tab.
  const settings = useShellState((s) => s.settings.open);
  const tools = useConnectors((s) => s.open);
  const toolsTab = useMarket((s) => s.tab);
  const overview = useProjects((s) => s.open);
  const libraries = useLibraries((s) => s.open);
  const page = settings ? (
    <SettingsPage />
  ) : overview ? (
    <div className="view">
      <ProjectsPage />
    </div>
  ) : tools ? (
    <div className="view">
      {toolsTab === "skills" ? (
        <SkillsPage />
      ) : toolsTab === "marketplace" ? (
        <MarketplacePage />
      ) : (
        <ConnectorsPage />
      )}
    </div>
  ) : libraries ? (
    <div className="view">
      <LibrariesPage />
    </div>
  ) : null;
  // Ctrl F finds in the open conversation or the File tab (pages over the work view have their own).
  useFindKeys(page !== null || covered);
  return (
    <main className={`content${page === null ? "" : " has-page"}`} id="content">
      <CoveredByPage.Provider value={page !== null}>
        <div className="content-base" inert={page !== null}>
          {covered ? (
            <>
              <div />
              <div className="view">
                <SetupScreen />
              </div>
            </>
          ) : selected !== null && session !== undefined ? (
            <>
              <Header title={session.title} sessionId={session.id} />
              <div className="view view-tx">
                <ServerNote />
                <Transcript key={session.id} sessionId={session.id} />
                <KeyNotice />
                {/* read-only while another process holds the session. */}
                <SessionHeldNote sessionId={session.id} />
                <HeldWrap sessionId={session.id}>
                  <LazyComposer key={`c-${session.id}`} sessionId={session.id} />
                </HeldWrap>
              </div>
            </>
          ) : (
            <>
              <div className="shead">
                <FoldChatButton />
                <h1>{TEXT.newChat}</h1>
                <NewChatToggles />
              </div>
              <div className="view view-welcome">
                {/* The calm idle state: a faint construction grid fading out from the centre; nothing moves. */}
                <div className="welcome-grid" aria-hidden="true" />
                <div className="welcome">
                  <div className="welcome-intro">
                    <BrandMark />
                    <h2>What should Vibe work on?</h2>
                    <p>
                      Type below to start a chat in the folder, pick one from the list, or press{" "}
                      {formatKeys("Mod+K")} for everything else.
                    </p>
                    <KeyNotice />
                    <SendToOffer />
                    <OrphanList />
                    <ServerNote />
                  </div>
                  <NewChatPanel />
                  <div className="new-chat-composer">
                    <LazyComposer sessionId={NEW_CHAT} />
                  </div>
                </div>
              </div>
            </>
          )}
        </div>
      </CoveredByPage.Provider>
      {page !== null && <div className="content-page">{page}</div>}
      <EndDialog />
    </main>
  );
}
