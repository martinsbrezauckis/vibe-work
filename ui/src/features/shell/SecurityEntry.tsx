/** Grouped guard decisions in the bell. Uses shared disclosure and links; opens the owning chat and step. */
import { useState } from "react";
import { LinkButton, RowDisclosure } from "../../components/Controls";
import { Icon } from "../../components/Icon";
import { GUARD_NAMES, type GuardKind } from "../../state/notifications";
import type { Toast } from "../../state/toastStore";
import { useApp } from "./AppContext";

/** One closed-by-default disclosure, with the newest fifty decisions. */
export function SecurityEntry({ toast, onNavigate }: { toast: Toast; onNavigate: () => void }) {
  const app = useApp();
  const [expanded, setExpanded] = useState(false);
  const group = toast.security;
  if (group === undefined) return null;
  const counts = Object.entries(group.counts)
    .sort((a, b) => b[1] - a[1])
    .map(([kind, count]) => `${GUARD_NAMES[kind as GuardKind]} ${String(count)}`)
    .join(" · ");
  const open = async (stepId: string | null) => {
    if (toast.sessionId === undefined) return;
    onNavigate();
    app.toasts.close();
    app.shell.actions.closeSettings();
    await app.sessions?.open(toast.sessionId);
    if (stepId !== null) app.find.actions.openStep(toast.sessionId, stepId);
  };
  return (
    <div className="security-entry">
      <RowDisclosure
        data-testid="shell.security-disclosure"
        aria-expanded={expanded}
        aria-controls={`guard-lines-${String(toast.id)}`}
        onClick={() => {
          setExpanded(!expanded);
        }}
      >
        <span className="security-entry-summary">
          <span className="security-entry-title ui-body" title={toast.text}>
            {toast.text}
          </span>
          <span className="ui-caption security-counts">{counts}</span>
        </span>
        <Icon name={expanded ? "chevron-down" : "chevron-right"} size={14} />
      </RowDisclosure>
      {expanded && (
        <div id={`guard-lines-${String(toast.id)}`} className="security-lines">
          {[...group.refusals]
            .sort((a, b) => b.at - a.at)
            .slice(0, 50)
            .map((line) => (
              <div className="security-line" key={line.refusalId} data-item-id={line.refusalId}>
                <time className="ui-caption" dateTime={new Date(line.at).toISOString()}>
                  {new Date(line.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                </time>
                <span className="ui-caption">{GUARD_NAMES[line.guard]}</span>
                <span className="security-what ui-body" title={line.what}>
                  {line.what}
                </span>
                <LinkButton
                  data-testid="shell.security-open"
                  data-item-id={line.refusalId}
                  onClick={() => {
                    void open(line.stepId);
                  }}
                >
                  {line.stepId === null ? "Open chat" : "Open step"}
                </LinkButton>
              </div>
            ))}
          {group.total > 50 && (
            <div className="security-more">
              <span className="ui-caption">and {String(group.total - 50)} more in this chat</span>
              <LinkButton
                data-testid="shell.security-more"
                onClick={() => {
                  void open(null);
                }}
              >
                Open chat
              </LinkButton>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
