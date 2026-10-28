/**
 * The "Add MCP server" address check, the same rule the core and the gate apply
 * (`crates/vw-gate/src/mcp_url.rs`), so the dialog can say why before anything is sent: `https://` to any
 * host, `http://` only to this computer (exactly 127.0.0.1, [::1] or localhost), no user name, no `#` part,
 * no `?` part (a key in the address would be stored in Vibe's config in clear), no spaces, control
 * characters or backslashes, at most 2048 characters, and the address must already be in its plain form
 * (the address Vibe parses must be the one checked; only the case of scheme and
 * host, a default port and the `/` of an empty path may differ). The core sends the canonical form.
 * Plus the notes the dialog shows: a vendor that only allows approved clients (`meta.GATED`).
 * Depends on: the WHATWG `URL` parser, model/meta. Called by: AddServerDialog, the connectors store.
 * Invariant: this is advice for the user; the core and the gate check again and are the authority.
 */
import { type GatedVendor, gatedVendor } from "./meta";

/** Longest address (the Rust rule has the same limit). */
const MAX_SERVER_URL = 2048;

/** The verdict on an address. */
export type ServerUrlCheck =
  | { ok: true; url: string; host: string; local: boolean; gated: GatedVendor | null }
  | { ok: false; reason: string };

function isLoopback(host: string): boolean {
  return host === "localhost" || host === "127.0.0.1" || host === "[::1]";
}

/** `text` with only the trivial normalisations (see the module docs), to compare with the parser's form. */
function trivial(text: string): string | null {
  const at = text.indexOf("://");
  if (at < 0) return null;
  const scheme = text.slice(0, at).toLowerCase();
  const rest = text.slice(at + 3);
  const slash = rest.indexOf("/");
  let authority = (slash < 0 ? rest : rest.slice(0, slash)).toLowerCase();
  const path = slash < 0 ? "/" : rest.slice(slash);
  const port = scheme === "https" ? ":443" : ":80";
  if (authority.endsWith(port)) authority = authority.slice(0, -port.length);
  return `${scheme}://${authority}${path}`;
}

/** Checks `text` (trimmed by the caller) against the rule in the module docs. */
export function checkServerUrl(text: string): ServerUrlCheck {
  if (
    text === "" ||
    text.length > MAX_SERVER_URL ||
    // eslint-disable-next-line no-control-regex
    /[\s\u0000-\u001f\u007f-\u009f\\]/.test(text)
  ) {
    return {
      ok: false,
      reason: "The address is empty, too long, or contains spaces or backslashes.",
    };
  }
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return { ok: false, reason: "This is not a complete web address (start with https://)." };
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    return {
      ok: false,
      reason: "The address must start with https:// (http:// only for this computer).",
    };
  }
  if (url.username !== "" || url.password !== "" || text.includes("@")) {
    return { ok: false, reason: "The address hides a user name or password before the host." };
  }
  if (url.hash !== "" || text.includes("#")) {
    return { ok: false, reason: "The address must not have a # part." };
  }
  if (url.search !== "" || text.includes("?")) {
    return {
      ok: false,
      reason:
        "Addresses with a ? part are not supported yet: a key in the address would be stored in Vibe's config in clear.",
    };
  }
  if (url.hostname === "") return { ok: false, reason: "The address has no host." };
  if (trivial(text) !== url.href) {
    return {
      ok: false,
      reason:
        "Type the address in its plain form: no encoded characters, ./.. parts, IP shorthands or non-ASCII host names.",
    };
  }
  const local = isLoopback(url.host.replace(/:\d+$/, ""));
  if (url.protocol === "http:" && !local) {
    return {
      ok: false,
      reason: "http:// is allowed only for a server on this computer; use https://.",
    };
  }
  return { ok: true, url: url.href, host: url.hostname, local, gated: gatedVendor(url.hostname) };
}
