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
import { goHome } from "./home";
import { PluginNoticeText } from "./PluginNoticeText";

const RESERVATION = "notifications-panel";

/** A nonmodal, top-layer popover; Show all and OS activation use the same header anchor. */
export function NotificationsPanel() {
  const app = useApp();
  const { toasts, sessions } = app;
  const history = useStore(toasts.history, (state) => state);
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
    element
      .querySelector<HTMLButtonElement>("[aria-label='Close']")
      ?.focus({ preventScroll: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(element);
    if (trigger) observer?.observe(trigger);
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        toasts.close();
      }
    };
    const outside = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        !element.contains(event.target) &&
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
    panel.current
      ?.querySelector(`[data-notification-id="${String(state.selectedId)}"]`)
      ?.scrollIntoView({ block: "start" });
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
        <div className="toast-dialog-actions">
          <Button
            data-testid="shell.notifications-panel-focus"
            small
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
            aria-label="Close"
            title="Close notifications"
            onClick={() => {
              toasts.close();
            }}
          />
        </div>
      </div>
      <div className="toast-dialog-body">
        {history.length === 0 && (
          <EmptyState glyph="chat" small>
            No notifications
          </EmptyState>
        )}
        {[...history].reverse().map((toast) => (
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
            <span className="toast-dialog-text ui-body">
              {toast.plugin ? (
                <PluginNoticeText text={toast.text} name={toast.plugin.name} />
              ) : (
                toast.text
              )}
            </span>
            <div className="toast-dialog-actions">
              {toast.secretLeak !== undefined && (
                <SecretLeakActions findingId={toast.secretLeak.findingId} />
              )}
              {toast.plugin !== undefined &&
                (toast.plugin.managed ? (
                  <>
                    <Button
                      small
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
                      small
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
                  small
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
              {toast.sessionId !== undefined && (
                <Button
                  data-testid="shell.notifications-panel-open"
                  small
                  onClick={() => {
                    if (toast.sessionId !== undefined) void sessions?.open(toast.sessionId);
                    toasts.close();
                  }}
                >
                  {term("workNames", "session").tooltip}
                </Button>
              )}
              <Button
                data-testid="shell.notifications-panel-closest"
                small
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
