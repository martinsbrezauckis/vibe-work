/**
 * Links in model output: a click never navigates the main window. The URL is
 * first shown in full in the app's own dialog, in its canonical form (`URL.href`: an internationalised
 * host appears as punycode, so a lookalike domain cannot hide behind familiar letters, and the link
 * text plays no part), then the user may open it in the system browser through the core
 * (`open_link`, which validates again in Rust: `crates/vw-platform/src/open_url.rs`).
 * Depends on: the WHATWG `URL` parser. Called by: the link dialog, the transcript store.
 * Invariants: only `http:` and `https:` can be opened; a URL with a user name or password is refused
 * (`https://bank.example@evil.example/` reads like the bank but goes to evil.example); control
 * characters and whitespace are refused.
 */

/** Longest URL that may be opened (the Rust side has the same limit). */
export const MAX_URL_LENGTH = 4096;

/** The verdict on a link. */
export type LinkCheck =
  { ok: true; url: string; host: string } | { ok: false; url: string; reason: string };

/** Checks `href` and returns its canonical form. */
export function checkLink(href: string): LinkCheck {
  // eslint-disable-next-line no-control-regex
  if (/[\s\u0000-\u001f\u007f-\u009f]/.test(href)) {
    return { ok: false, url: href, reason: "The link contains spaces or control characters." };
  }
  if (href.length > MAX_URL_LENGTH) {
    return { ok: false, url: href.slice(0, 200), reason: "The link is too long to open." };
  }
  let parsed: URL;
  try {
    parsed = new URL(href);
  } catch {
    return { ok: false, url: href, reason: "This is not a complete web address." };
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return { ok: false, url: parsed.href, reason: "Only web links (http, https) can be opened." };
  }
  if (parsed.username !== "" || parsed.password !== "") {
    return {
      ok: false,
      url: parsed.href,
      reason: "The link hides a user name before the real address, a common disguise. Not opened.",
    };
  }
  if (parsed.hostname === "") {
    return { ok: false, url: parsed.href, reason: "The link has no host." };
  }
  return { ok: true, url: parsed.href, host: parsed.hostname };
}
