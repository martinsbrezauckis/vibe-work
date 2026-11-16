/**
 * The app's two announcement regions (`state/announcer.ts`): polite and assertive, always in the page so the first
 * message is read too, outside the window's layout and never made inert by a modal dialog (`data-announcer-root`).
 * Depends on: the announcer store. Called by: Shell.
 * Invariants: invisible (`.sr`); nothing else in the app adds an app-level live region; each region's text content
 * is its current message on every system. On the Linux web view a message said while the window did not have the
 * focus (the app's own native confirmation window was active), or just after it got the focus back, is said again:
 * first the focused element takes the focus again, then the message follows. Orca presents a page's live region
 * only while its page script is active; when a native GTK window of the app closes, the main window's activation
 * keeps the GTK script active and WebKitGTK reports no focus in the page, so only a focus change in the page makes
 * Orca present the page (and its regions) again.
 */
import { type ReactElement, memo, useEffect } from "react";
import { useStore } from "../../state/createStore";
import { announcerStore, hasUnheard, holdUnheard, sayUnheard } from "../../state/announcer";

/** How long after the window gets the focus back the focused element takes the focus again (ms): after the reader
 * has presented the window. Messages said until then count as unheard. */
const REFOCUS_MS = 500;
/** How long after that focus change an unheard message is said again (ms): the reader reports the focus change
 * (and presents the focused element) first, so the message is presented after it. */
const CLAIM_MS = 250;

/** The focused element takes the focus again (losing it first), so the web view reports a focus change in the page.
 * When the page itself has the focus (the control lost it while it was disabled, as Settings disables its controls
 * while a confirmation is open), `previous`, the element that lost the focus last, takes it back if it still can. */
function claimFocus(previous: HTMLElement | null): void {
  const focused = document.activeElement;
  if (focused instanceof HTMLElement && focused !== document.body) {
    focused.blur();
    focused.focus({ preventScroll: true });
    return;
  }
  if (previous?.isConnected === true) previous.focus({ preventScroll: true });
}

/**
 * Whether the regions are read-only text fields. The Linux web view (WebKitGTK) tells screen readers nothing when a
 * paragraph's text changes, and Orca speaks live regions only from text-change events; a read-only text field does
 * report its text change. Elsewhere (WebView2, WKWebView) the regions stay paragraphs.
 */
export function regionsAreTextFields(userAgent: string): boolean {
  return /\bLinux\b/.test(userAgent) && !/\bAndroid\b/.test(userAgent);
}

/** One region. As a text field it is unnamed, read-only, outside the tab order and hidden from the reading order
 * (`aria-hidden`: WebKitGTK still reports its text change, which Orca speaks, but browse mode no longer meets an empty
 * "entry"). Its text is set as the default value (the field's text content), which its value follows. Memoized: a
 * window re-render must not set the same text again, which a reader would report as new text and say twice. */
const Region = memo(function Region({
  live,
  text,
  field,
}: {
  live: "polite" | "assertive";
  text: string;
  field: boolean;
}): ReactElement {
  if (field) {
    return (
      <textarea
        readOnly
        tabIndex={-1}
        aria-hidden="true"
        aria-live={live}
        aria-atomic="true"
        data-announcer={live}
        data-testid={`shell.announcer-${live}`}
        defaultValue={text}
      />
    );
  }
  return (
    <p aria-live={live} aria-atomic="true" data-announcer={live}>
      {text}
    </p>
  );
});

/** The regions. */
export function Announcer({
  userAgent = navigator.userAgent,
}: {
  userAgent?: string;
}): ReactElement {
  const polite = useStore(announcerStore, (s) => s.polite);
  const assertive = useStore(announcerStore, (s) => s.assertive);
  const field = regionsAreTextFields(userAgent);
  useEffect(() => {
    if (!field) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let previous: HTMLElement | null = null;
    const left = (event: FocusEvent) => {
      if (event.target instanceof HTMLElement) previous = event.target;
    };
    const refocused = () => {
      clearTimeout(timer);
      holdUnheard(REFOCUS_MS);
      timer = setTimeout(() => {
        if (!hasUnheard()) return;
        claimFocus(previous);
        timer = setTimeout(sayUnheard, CLAIM_MS);
      }, REFOCUS_MS);
    };
    document.addEventListener("focusout", left, true);
    window.addEventListener("focus", refocused);
    return () => {
      document.removeEventListener("focusout", left, true);
      window.removeEventListener("focus", refocused);
      clearTimeout(timer);
    };
  }, [field]);
  return (
    <div className="sr" data-announcer-root="">
      <Region live="polite" text={polite} field={field} />
      <Region live="assertive" text={assertive} field={field} />
    </div>
  );
}
