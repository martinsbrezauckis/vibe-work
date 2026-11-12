/** No OS notification for the request the person is looking at, also while the app's own confirmation window has
 * the focus (the webview then reports no focus); the host decides "the app is in front" from the OS. Synthetic data. */
import { afterEach, describe, expect, it, vi } from "vitest";
import { type CoreState, initialCore } from "../../../ui/src/state/coreModel";
import { createStore } from "../../../ui/src/state/createStore";
import { createNotifier } from "../../../ui/src/state/notifyStore";
import { createToasts } from "../../../ui/src/state/toastStore";
import { fakeClient } from "./support/fakeClient";

/** A session "s1" whose subagent waits on an approval card, or not yet. */
function state(waiting: boolean): CoreState {
  return {
    ...initialCore,
    sessions: {
      s1: {
        id: "s1",
        session: { id: "s1", title: "Review", status: { type: "running" } },
        localOnly: false,
        openCallbackIds: [],
        state: {
          childSessions: [
            {
              id: "child",
              name: "Reviewer",
              status: waiting
                ? { type: "blocked", callbackId: "approval" }
                : { type: "idle" },
            },
          ],
        },
      },
    },
    subagentCards: waiting
      ? {
          approval: {
            sessionId: "child",
            parentSessionId: "s1",
            entry: { callbackId: "approval", type: "callback" },
          },
        }
      : {},
  };
}

/** Session "s1" with the agent's request shown in the app's own confirmation window (a native ask), or not yet. */
function nativeState(waiting: boolean): CoreState {
  const base = state(false);
  return waiting
    ? {
        ...base,
        asks: {
          "ask-1": {
            id: "ask-1",
            kind: "vibe-work/gitGuard",
            session: "s1",
            requester: "agent",
            surface: "native",
            title: "this Git command",
            openedAt: 1000,
            offers: [],
            personOnly: true,
            everyCall: false,
            alwaysSetting: null,
          },
        },
      }
    : base;
}

/** The OS notifications asked of the host when the request arrives with this webview focus and open session;
 * `override` uses the Debug API's test hook instead of the real signal; `native` = the request is shown in the
 * app's own confirmation window instead of a card. */
function arrive(
  focused: boolean,
  openSession: string,
  override = false,
  native = false,
) {
  const before = native ? nativeState : state;
  const store = createStore(before(false));
  const client = fakeClient();
  const notifier = createNotifier({ persist: false });
  if (override) notifier.setFocusOverride(focused);
  else vi.spyOn(document, "hasFocus").mockReturnValue(focused);
  const toasts = createToasts();
  const stop = notifier.watch(
    { store, client },
    () => openSession,
    toasts.record,
  );
  store.set(before(true));
  stop();
  return client.notifications;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("OS notifications and the app in front", () => {
  it("asks the host to skip the toast when the app is in front, if only the webview's focus is missing", () => {
    // The open session is this one: the webview lost focus (e.g. to the app's own confirmation window), so only
    // the host can tell whether the person is looking at the app.
    const sent = arrive(false, "s1");
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({
      sessionId: "s1",
      skipWhenAppInFront: true,
    });
  });

  it("also leaves another chat's request to the host while the webview has no focus", () => {
    // The app's own confirmation window for that request may be what has the focus (seen on Windows: the new chat on screen).
    const sent = arrive(false, "other");
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({
      sessionId: "s1",
      skipWhenAppInFront: true,
    });
  });

  it("shows the toast for another chat while the person is in the app's window", () => {
    const sent = arrive(true, "other");
    expect(sent).toHaveLength(1);
    expect(sent[0]?.skipWhenAppInFront).not.toBe(true);
  });

  it("asks for nothing while the webview has focus on that session", () => {
    expect(arrive(true, "s1")).toHaveLength(0);
  });

  it("leaves a request in the app's own confirmation window to the host, also for a chat not on screen", () => {
    // The confirmation window comes in front of the app by itself: while the app (or that window) is in front the
    // toast would only repeat it; behind another program the host still shows it.
    const sent = arrive(true, "other", false, true);
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({
      sessionId: "s1",
      skipWhenAppInFront: true,
    });
  });

  it("leaves the Debug API's focus override as it is (a test that says unfocused gets its toast)", () => {
    const sent = arrive(false, "s1", true);
    expect(sent).toHaveLength(1);
    expect(sent[0]?.skipWhenAppInFront).not.toBe(true);
  });
});
