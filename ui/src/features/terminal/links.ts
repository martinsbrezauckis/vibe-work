/**
 * What happens when the user activates a link in terminal output (the web-links addon's URLs and
 * OSC 8 hyperlinks a program prints). Terminal output is untrusted: only `http(s)` links
 * go further, and they go to the same flow as transcript links: the link dialog shows
 * the full canonical URL and its host, and only its "Open in browser" button hands the URL to the
 * core (`open_link`, validated again in Rust), which opens the system browser.
 * Depends on: nothing. Called by: session.ts (link handler), register.tsx (installs the opener:
 * `app.transcript.askLink`, which shows the `LinkDialog`), tests.
 * Invariants: `javascript:`, `file:`, `data:` and every other scheme are refused here, before the
 * dialog; nothing opens without the dialog's confirmation; without an opener nothing happens; the
 * main window never navigates (the navigation guard checks again).
 */

/** Asks the user about a checked `http(s)` URL (the link dialog). */
export type LinkOpener = (url: string) => void;

let opener: LinkOpener | null = null;

/** Installs the opener. Returns the function that removes it. */
export function setLinkOpener(next: LinkOpener): () => void {
  opener = next;
  return () => {
    if (opener === next) opener = null;
  };
}

/** The URL when it is an absolute `http:` or `https:` URL, else `null`. */
export function safeLink(text: string): string | null {
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return null;
  }
  return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
}

/**
 * Handles an activated link: refuses unsafe schemes and, for `http(s)`, asks the user through the
 * opener (the link dialog). Returns what was done.
 */
export function activateLink(text: string): "refused" | "asked" {
  const url = safeLink(text);
  if (url === null || opener === null) return "refused";
  opener(url);
  return "asked";
}
