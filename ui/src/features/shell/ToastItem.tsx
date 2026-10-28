/**
 * One readable toast: a three-line prefix with an explicit full-text action, independent dismiss
 * control and length-aware timer. Docked in the status bar it shows its own message on one line (cut with an
 * ellipsis) with Show all. Depends on toastReading/Icon; called by Toasts. Pointer, keyboard
 * focus and an open notifications panel pause expiry; warnings and errors persist.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { Button, IconButton } from "../../components/Controls";
import type { Toast } from "../../state/toastStore";
import { initialToastPreview, measureToastPreview, toastLifetime } from "../../state/toastReading";
import { PluginNoticeText } from "./PluginNoticeText";

const EXIT_MS = 160;

function reducedMotion(): boolean {
  return (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** Renders an active toast; full text is opened through its retained history id. */
export function ToastItem({
  toast,
  docked,
  active,
  reading,
  onGone,
  onOpen,
}: {
  toast: Toast;
  docked: boolean;
  active: boolean;
  reading: boolean;
  onGone: () => void;
  onOpen: () => void;
}) {
  const { id, kind, text } = toast;
  const life = toastLifetime(toast);
  const [leaving, setLeaving] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [preview, setPreview] = useState(() => initialToastPreview(text));
  const message = useRef<HTMLSpanElement>(null);
  const paused = hovered || focused || reading || (docked && !active);
  const gone = useRef(onGone);
  const exitTimer = useRef<number | null>(null);
  const left = useRef(life === null ? null : life - EXIT_MS);
  useEffect(() => {
    gone.current = onGone;
  }, [onGone]);
  useEffect(
    () => () => {
      if (exitTimer.current !== null) window.clearTimeout(exitTimer.current);
    },
    [],
  );
  useLayoutEffect(() => {
    if (docked || message.current === null) return;
    const element = message.current;
    const measure = () => {
      setPreview(measureToastPreview(text, element));
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(element);
    return () => {
      observer?.disconnect();
    };
  }, [text, docked]);
  const leave = useCallback(() => {
    if (exitTimer.current !== null || leaving) return;
    if (reducedMotion()) {
      gone.current();
      return;
    }
    setLeaving(true);
    exitTimer.current = window.setTimeout(() => {
      gone.current();
    }, EXIT_MS);
  }, [leaving]);
  useEffect(() => {
    if (left.current === null || paused || leaving) return;
    const started = Date.now();
    const timer = window.setTimeout(leave, left.current);
    return () => {
      window.clearTimeout(timer);
      if (left.current !== null) left.current = Math.max(0, left.current - (Date.now() - started));
    };
  }, [paused, leaving, leave]);
  return (
    <div
      data-item-id={toast.id}
      className={`toast${leaving ? " out" : ""}`}
      data-kind={kind}
      data-guard={toast.guardRefusal === undefined ? undefined : "true"}
      data-toast-id={String(id)}
      data-active={active ? "true" : undefined}
      hidden={docked && !active}
      onMouseEnter={() => {
        setHovered(true);
      }}
      onMouseLeave={() => {
        setHovered(false);
      }}
      onFocusCapture={() => {
        setFocused(true);
      }}
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false);
      }}
    >
      <span
        className={`px ${kind === "error" ? "err" : kind === "warn" ? "warn" : "ok"}`}
        aria-hidden="true"
      />
      <div className="toast-content">
        <span ref={message} className="toast-text">
          {toast.plugin ? (
            <PluginNoticeText text={docked ? text : preview} name={toast.plugin.name} />
          ) : docked ? (
            text
          ) : (
            preview
          )}
        </span>
        {(toast.guardRefusal !== undefined || docked || preview !== text) && (
          <Button
            data-testid="shell.toast-item-button"
            appearance="expander"
            data-toast-more="true"
            aria-label={toast.guardRefusal === undefined ? "Open notifications: Show all" : "Show"}
            onClick={onOpen}
          >
            {toast.guardRefusal === undefined ? "Show all" : "Show"}
          </Button>
        )}
      </div>
      <IconButton
        tooltip={false}
        data-testid="shell.toast-item-x"
        icon="x"
        label="Dismiss"
        onClick={leave}
      />
      {life !== null && (
        <span
          className="toast-timer"
          aria-hidden="true"
          style={{ animationDuration: `${String(life)}ms` }}
          data-paused={paused ? "true" : undefined}
        />
      )}
    </div>
  );
}
