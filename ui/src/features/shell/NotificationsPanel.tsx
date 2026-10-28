/**
 * Complete retained notifications in a header-anchored popover. Depends on shared controls,
 * AppContext and the native layer; called by Toasts. Long text scrolls and wraps, Close stays
 * visible, native-view space is reserved, and Escape/Close return focus to the opener.
 */
import { SecretLeakActions } from "../secrets/SecretLeakActions";
import { term } from "../../vocabulary";
import { useLayoutEffect, useRef } from "react";
import { createPortal } from "react-dom";

import { Button, IconButton } from "../../components/Controls";
import { EmptyState } from "../../components/EmptyState";
import { reserve } from "../../components/nativeLayer";
import { useStore } from "../../state/createStore";
import { computeLayout } from "../../state/panelLayout";
import { useApp } from "./AppContext";
import { Selector } from "../../components/Selector";
import type { BellKind } from "../../state/notifications";
import { SecurityEntry } from "./SecurityEntry";
import { goHome } from "./home";
import { showChatPrompts } from "../ask/waiting";
import { PluginNoticeText } from "./PluginNoticeText";

const FILTERS = [
  { value: "all", label: "All", empty: "No notifications" },
  { value: "security", label: "Security", empty: "No security entries." },
  { value: "approval", label: "Approvals", empty: "No approvals." },
  { value: "finished", label: "Finished", empty: "No finished chats." },
  { value: "failure", label: "Failures", empty: "No failures." },
  { value: "worktrees", label: "Session copies", empty: "No session copies." },
] as const;
const RESERVATION = "notifications-panel";

/** A nonmodal, top-layer popover; Show all and OS activation use the same header anchor. */
export function NotificationsPanel() {
  const app = useApp();
  const { toasts, sessions } = app;
  const history = useStore(toasts.history, (state) => state);
  const filter = useStore(toasts.filter, (value) => value);
  const visible = history.filter((notice) => filter === "all" || notice.bellKind === filter);
  const state = useStore(toasts.panel, (value) => value);
  const panel = useRef<HTMLDivElement>(null);
  const restoreFocus = useRef(true);
  useLayoutEffect(() => {
    const element = panel.current;
    if (element === null || !state.open) return;
    const trigger = document.getElementById("notifications-trigger");
    const active = document.activeElement;
    const opener =
      active instanceof HTMLElement && (active === trigger || active.matches("[data-toast-more]"))
        ? active
        : trigger;
    restoreFocus.current = true;
    element.showPopover(trigger ? { source: trigger } : undefined);
    const measure = () => {
      const anchor = trigger?.getBoundingClientRect();
      const inset = 8;
      const top = anchor ? anchor.bottom + inset : 46;
      element.style.top = `${String(top)}px`;
      element.style.maxHeight = `${String(Math.min(420, Math.max(0, window.innerHeight - top - 34)))}px`;
      const width = element.getBoundingClientRect().width;
      const left = Math.max(
        inset,
        Math.min(
          (anchor?.right ?? window.innerWidth - inset) - width,
          window.innerWidth - width - inset,
        ),
      );
      element.style.left = `${String(left)}px`;
      reserve(RESERVATION, element.getBoundingClientRect());
    };
    measure();
    element.focus({ preventScroll: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(element);
    if (trigger) observer?.observe(trigger);
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !document.querySelector("[data-selector-trigger]")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        toasts.close();
      }
    };
    const outside = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        !element.contains(event.target) &&
        !(event.target instanceof Element && event.target.closest("[data-selector-trigger]")) &&
        !trigger?.contains(event.target)
      ) {
        restoreFocus.current = false;
        toasts.close();
      }
    };
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    document.addEventListener("keydown", keydown, true);
    document.addEventListener("pointerdown", outside);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
      document.removeEventListener("keydown", keydown, true);
      document.removeEventListener("pointerdown", outside);
      element.hidePopover();
      reserve(RESERVATION, null);
      if (restoreFocus.current) {
        if (opener?.isConnected) opener.focus({ preventScroll: true });
        else trigger?.focus({ preventScroll: true });
      }
    };
  }, [state.open, toasts]);
  useLayoutEffect(() => {
    const row = panel.current?.querySelector<HTMLElement>(
      `[data-notification-id="${String(state.selectedId)}"]`,
    );
    row?.scrollIntoView({ block: "start" });
    row?.querySelector<HTMLButtonElement>(".ui-disclosure")?.focus({ preventScroll: true });
  }, [state.open, state.selectedId]);
  if (!state.open) return null;
  return createPortal(
    <div
      data-testid="shell.notifications-panel-toast-dialog"
      ref={panel}
      id="notifications-panel"
      className="toast-dialog ui-panel"
      role="dialog"
      aria-labelledby="notifications-heading"
      popover="manual"
      tabIndex={-1}
      onBlur={(event) => {
        if (
          event.relatedTarget instanceof Node &&
          !event.currentTarget.contains(event.relatedTarget) &&
          !(
            event.relatedTarget instanceof Element &&
            event.relatedTarget.closest("[data-selector-trigger]")
          ) &&
          event.relatedTarget !== document.getElementById("notifications-trigger")
        ) {
          restoreFocus.current = false;
          toasts.close();
        }
      }}
    >
      <div className="toast-dialog-head">
        <h2 id="notifications-heading" className="ui-heading">
          Notifications
        </h2>
        <label className="security-filter">
          Show{" "}
          <Selector<"all" | BellKind>
            label="Show"
            value={filter}
            options={FILTERS}
            onChange={(value) => {
              toasts.filter.set(value);
            }}
            testId="shell.notifications-show"
            compact
          />
        </label>
        <div className="toast-dialog-actions">
          <Button
            data-testid="shell.notifications-panel-focus"
            shape="outlined"
            disabled={history.length === 0}
            onClick={() => {
              panel.current?.querySelector<HTMLButtonElement>("[aria-label='Close']")?.focus();
              toasts.clear();
            }}
          >
            Clear all
          </Button>
          <IconButton
            data-testid="shell.notifications-panel-close"
            icon="x"
            label="Close"
            title="Close notifications"
            onClick={() => {
              toasts.close();
            }}
          />
        </div>
      </div>
      <div className="toast-dialog-body">
        {visible.length === 0 && (
          <EmptyState glyph="chat" small>
            {FILTERS.find((option) => option.value === filter)?.empty}
          </EmptyState>
        )}
        {[...visible]
          .sort((a, b) => b.createdAt - a.createdAt)
          .map((toast) => (
            <article
              data-item-id={toast.id}
              className="toast-dialog-row ui-list-row"
              key={toast.id}
              data-kind={toast.kind}
              data-notification-id={String(toast.id)}
            >
              <time
                className="ui-caption"
                dateTime={new Date(toast.createdAt).toISOString()}
                title={new Date(toast.createdAt).toLocaleString()}
              >
                {new Date(toast.createdAt).toLocaleTimeString([], {
                  hour: "2-digit",
                  minute: "2-digit",
                })}
              </time>
              {toast.security === undefined ? (
                <span className="toast-dialog-text ui-body">
                  {toast.plugin ? (
                    <PluginNoticeText text={toast.text} name={toast.plugin.name} />
                  ) : (
                    toast.text
                  )}
                </span>
              ) : (
                <SecurityEntry
                  toast={toast}
                  onNavigate={() => {
                    restoreFocus.current = false;
                  }}
                />
              )}
              <div className="toast-dialog-actions">
                {toast.secretLeak !== undefined && (
                  <SecretLeakActions findingId={toast.secretLeak.findingId} />
                )}
                {toast.plugin !== undefined &&
                  (toast.plugin.managed ? (
                    <>
                      <Button
                        shape="outlined"
                        data-testid="shell.notifications-plugin-on"
                        onClick={() => {
                          if (toast.plugin) void app.market?.actions.showPlugin(toast.plugin.path);
                          toasts.close();
                        }}
                      >
                        Turn on
                      </Button>
                      <Button
                        variant="danger"
                        shape="outlined"
                        data-testid="shell.notifications-plugin-remove"
                        onClick={() => {
                          if (toast.plugin)
                            void app.market?.actions
                              .changePlugin("removePlugin", toast.plugin.path)
                              .then((changed) => {
                                if (changed) toasts.forget(toast.id);
                              });
                        }}
                      >
                        Remove
                      </Button>
                    </>
                  ) : (
                    <span className="cmuted">Managed elsewhere</span>
                  ))}
                {toast.action === "sessions" && (
                  <Button
                    data-testid="shell.notifications-panel-go-home"
                    shape="outlined"
                    onClick={() => {
                      goHome(app);
                      app.shell.actions.selectSession(null);
                      app.shell.actions.setFilter("");
                      app.shell.actions.showList();
                      if (computeLayout(app.shell.store.get(), window.innerWidth).drawer)
                        app.shell.actions.setDrawer(true);
                      void app.core?.setIncludeArchived(true);
                      toasts.close();
                    }}
                  >
                    Sessions
                  </Button>
                )}
                {toast.sessionId !== undefined && toast.security === undefined && (
                  <Button
                    data-testid="shell.notifications-panel-open"
                    shape="outlined"
                    onClick={() => {
                      if (toast.sessionId !== undefined) {
                        // Selecting another chat brings its prompt windows forward; the chat already shown needs it.
                        const shownAlready = app.shell.store.get().selected === toast.sessionId;
                        void sessions?.open(toast.sessionId);
                        if (shownAlready) showChatPrompts(app, toast.sessionId);
                      }
                      toasts.close();
                    }}
                  >
                    {term("workNames", "session").tooltip}
                  </Button>
                )}
                <Button
                  data-testid="shell.notifications-panel-closest"
                  shape="outlined"
                  aria-label={`Dismiss: ${toast.text}`}
                  onClick={(event) => {
                    const row = event.currentTarget.closest("article");
                    const adjacent = row?.nextElementSibling ?? row?.previousElementSibling;
                    const next =
                      adjacent?.querySelector<HTMLButtonElement>("[aria-label^='Dismiss:']") ??
                      panel.current?.querySelector<HTMLButtonElement>("[aria-label='Close']");
                    next?.focus();
                    toasts.forget(toast.id);
                  }}
                >
                  Dismiss
                </Button>
              </div>
            </article>
          ))}
      </div>
    </div>,
    document.body,
  );
}
