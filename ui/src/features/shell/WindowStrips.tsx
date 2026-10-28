/** Folded session and chat strips use the existing icon buttons and activity state. Called by Shell; swapping never changes the shared slot width. */
import { BADGE_LABEL } from "../../state/sessionSummary";
import { IconButton } from "../../components/Controls";
import { useApp } from "./AppContext";
import { useLayout, useSessionSummaries, useShellState } from "./hooks";
import { formatKeys } from "../../state/shortcuts";
import { TEXT } from "../../vocabulary";

export function SessionsStrip() {
  const app = useApp();
  const layout = useLayout();
  const selected = useShellState((s) => s.selected);
  const sessions = useSessionSummaries();
  return (
    <aside
      className="window-strip sessions-strip"
      data-toast-avoid="strip"
      aria-label="Folded sessions"
    >
      <IconButton
        data-testid="shell.unfold-sessions"
        icon="panel-left"
        label="Show sessions"
        title={`Show sessions (${formatKeys("Mod+B")})`}
        onClick={() => {
          if (layout.editorSplit) app.shell.actions.swapEditorSlot();
          else app.shell.actions.showList();
        }}
      />
      <div className="session-strip-items">
        {sessions
          .filter((s) => !s.archived)
          .map((s) => (
            <IconButton
              key={s.id}
              data-testid="shell.strip-session"
              data-item-id={s.id}
              icon="message-square"
              text={s.title
                .split(/\s+/)
                .slice(0, 2)
                .map((w) => w[0])
                .join("")
                .toUpperCase()}
              label={`Open ${s.title} · ${BADGE_LABEL[s.badge]}`}
              title={`${s.title} · ${BADGE_LABEL[s.badge]}`}
              data-activity={s.badge}
              aria-pressed={s.id === selected}
              onClick={() => {
                app.shell.actions.unfoldChat();
                void app.sessions?.open(s.id);
              }}
            />
          ))}
      </div>
    </aside>
  );
}
export function ChatStrip() {
  const app = useApp();
  const selected = useShellState((s) => s.selected);
  const session = useSessionSummaries().find((s) => s.id === selected);
  return (
    <aside className="window-strip chat-strip" data-toast-avoid="strip" aria-label="Folded chat">
      <IconButton
        data-testid="shell.unfold-chat"
        icon="panel-left"
        label="Show chat"
        title="Show chat (Escape)"
        onClick={app.shell.actions.unfoldChat}
      />
      <span className="chat-strip-title">{session?.title ?? TEXT.newChat}</span>
      <span className="chat-strip-state">{BADGE_LABEL[session?.badge ?? "idle"]}</span>
    </aside>
  );
}
export function FoldChatButton() {
  const app = useApp();
  const layout = useLayout();
  return (
    <IconButton
      data-testid="shell.fold-chat"
      icon="panel-left"
      label="Fold chat"
      disabled={!layout.panelOpen}
      title={`Fold chat (${formatKeys("Mod+Alt+B")})`}
      onClick={app.shell.actions.foldChat}
    />
  );
}
