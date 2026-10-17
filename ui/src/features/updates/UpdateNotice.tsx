/**
 * The update notice: a small card under the title bar whenever a newer version exists (available,
 * downloading, ready to install, or a failed download / install). It says what happens in plain words and offers
 * the one next step; "Install and restart" is only a request to the host, which installs nothing without it.
 * the card sits at the top right, over the side panel, where a native view (preview,
 * viewers, agent browser) would paint over it; while it shows it reserves its rectangle (`nativeLayer`) and the
 * native views clip around it.
 * the update area also holds the Vibe card (`VibeUpdate.tsx`); both cards sit in one fixed stack, and
 * the stack (not each card) reserves its rectangle while it has content.
 * Depends on: AppContext, state/updates, hooks, components/nativeLayer, VibeUpdate, ReleaseHost. Called by: Shell.
 * Invariants: renders nothing in a build without an update channel, while idle, for a failed check, or after
 * "Later" for the same state; the card is a polite status region and never takes focus; its reservation lives
 * exactly as long as the card.
 */
import { Fragment, type RefObject, useLayoutEffect, useRef } from "react";

import { reserve } from "../../components/nativeLayer";
import { useStore } from "../../state/createStore";
import { type NoticeAction, noticeFor } from "../../state/updates";
import { useApp } from "../shell/AppContext";
import { ReleaseHost } from "./ReleaseHost";
import { VibeUpdateCard } from "./VibeUpdate";

const LABELS: Record<NoticeAction, string> = {
  download: "Download",
  install: "Install and restart",
  openReleasePage: "Open release page",
  retry: "Try again",
  later: "Later",
};

/** The update area: the Vibe Work notice and the Vibe card (each renders nothing when it has nothing to say). */
export function UpdateNotice() {
  const { updates } = useApp();
  const stack = useRef<HTMLDivElement>(null);
  useReservation(stack);
  return (
    // toasts keep clear of the update cards too (a top-right toast would cover them).
    <div className="update-notices" ref={stack} data-toast-avoid="updates">
      {updates !== null && <Card updates={updates} />}
      <VibeUpdateCard />
    </div>
  );
}

/** The reservation key of the card in `nativeLayer`. */
const RESERVATION = "update-notice";

/** Keeps the stack's rectangle reserved above native views while it shows a card (an empty stack reserves nothing). */
function useReservation(card: RefObject<HTMLDivElement | null>): void {
  useLayoutEffect(() => {
    const element = card.current;
    if (element === null) return;
    const update = () => {
      const r = element.getBoundingClientRect();
      reserve(
        RESERVATION,
        r.width < 1 || r.height < 1
          ? null
          : {
              x: Math.floor(r.left),
              y: Math.floor(r.top),
              width: Math.ceil(r.width),
              height: Math.ceil(r.height),
            },
      );
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    window.addEventListener("resize", update);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", update);
      reserve(RESERVATION, null);
    };
  }, [card]);
}

function Card({ updates }: { updates: NonNullable<ReturnType<typeof useApp>["updates"]> }) {
  const state = useStore(updates.store, (s) => s);
  if (!state.available || state.view === null) return null;
  const notice = noticeFor(state.view);
  if (notice === null || state.dismissed === notice.key) return null;
  return <Shown notice={notice} releasePage={state.view.releasePage} updates={updates} />;
}

/** The card itself (mounted only while it shows; the stack's reservation follows its size). */
function Shown({
  notice,
  releasePage,
  updates,
}: {
  notice: NonNullable<ReturnType<typeof noticeFor>>;
  /** The view's release page, for the host shown beside "Open release page". */
  releasePage: string | null;
  updates: NonNullable<ReturnType<typeof useApp>["updates"]>;
}) {
  const act = (action: NoticeAction): void => {
    const run =
      action === "download"
        ? updates.download
        : action === "install"
          ? updates.install
          : action === "openReleasePage"
            ? updates.openReleasePage
            : action === "retry"
              ? updates.check
              : null;
    if (run === null) updates.dismiss();
    else run().catch(() => undefined);
  };
  return (
    <div className="update-notice" role="status" aria-live="polite" data-tone={notice.tone}>
      <div className="update-notice-text">
        <strong>{notice.headline}</strong>
        {notice.detail !== null && <span className="d">{notice.detail}</span>}
        {notice.progress !== null && (
          <progress max={1} value={notice.progress} aria-label="Download progress" />
        )}
      </div>
      {notice.actions.length > 0 && (
        <div className="update-notice-actions">
          {notice.actions.map((action) => (
            <Fragment key={action}>
              <button
                data-testid="updates.update-notice-act"
                data-item-id={action}
                type="button"
                className={
                  action === "install" || action === "download" ? "tb-btn primary" : "tb-btn"
                }
                onClick={() => {
                  act(action);
                }}
              >
                {LABELS[action]}
              </button>
              {action === "openReleasePage" && <ReleaseHost url={releasePage} />}
            </Fragment>
          ))}
        </div>
      )}
    </div>
  );
}
