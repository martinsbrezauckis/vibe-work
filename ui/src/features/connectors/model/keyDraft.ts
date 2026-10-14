/**
 * The key part of the "Add MCP server" dialog, as plain data: which vault secret (an existing project
 * secret or a new one) and which header carries it. `keyChoice` turns the draft into the request's fields or says
 * what is missing, with the same header rule as the core (`vw-core-api/src/mcp_keys.rs`, `checked_header`).
 * Depends on: state/connectors (`KeyChoice`). Called by: KeySection, AddServerDialog, tests.
 * Invariants: the value never leaves the draft except inside `newSecret` of the one request (the dialog clears it).
 */
import type { KeyChoice } from "../../../state/connectors";

/** What the user filled in. */
export interface KeyDraft {
  /** `existing`: pick a vault secret; `new`: type a name and the value. */
  mode: "existing" | "new";
  secretId: string;
  newName: string;
  newValue: string;
  /** `bearer`: `Authorization: Bearer <key>`; `custom`: the key as is in `header`. */
  headerMode: "bearer" | "custom";
  header: string;
}

/** An empty draft (a new secret when the vault has none to pick). */
export function emptyDraft(hasSecrets: boolean): KeyDraft {
  return {
    mode: hasSecrets ? "existing" : "new",
    secretId: "",
    newName: "",
    newValue: "",
    headerMode: "bearer",
    header: "",
  };
}

/** Header names the core refuses (they belong to the connection itself). */
const RESERVED = new Set([
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "proxy-connection",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
  "host",
  "content-length",
  "content-type",
  "accept",
  "cookie",
  "mcp-session-id",
  "mcp-protocol-version",
]);

/** Why `header` cannot carry a key, or `null` when it can. */
export function headerProblem(header: string): string | null {
  const h = header.trim();
  if (h === "") return "Type the header's name, for example X-Api-Key.";
  if (h.length > 64 || !/^[A-Za-z0-9!#$%&'*+\-.^_`|~]+$/.test(h))
    return "A header name has up to 64 letters, digits and - _ . characters.";
  if (RESERVED.has(h.toLowerCase())) return "That header is used by the connection itself.";
  return null;
}

/** The request's key fields, or the sentence saying what is missing. */
export function keyChoice(draft: KeyDraft): KeyChoice | string {
  let header: string | null = null;
  if (draft.headerMode === "custom") {
    const problem = headerProblem(draft.header);
    if (problem !== null) return problem;
    header = draft.header.trim();
  }
  if (draft.mode === "existing") {
    if (draft.secretId === "") return "Pick the secret that holds the key.";
    return { secretId: draft.secretId, newSecret: null, header };
  }
  const name = draft.newName.trim();
  if (name === "") return "Give the new secret a name.";
  if (draft.newValue === "") return "Paste the key or token.";
  return { secretId: null, newSecret: { name, value: draft.newValue }, header };
}
