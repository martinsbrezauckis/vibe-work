/**
 * The covers words of a session grant (what "Allow for this session" lets Vibe do again), as the core builds them
 * (`SessionGrant.covers`, `CoversPart[]`: text runs, some shown as code). The page renders the parts; these helpers
 * give the plain text (search, sorting, dialog lists) and the row form whose first letter is upper case.
 * The text helpers are the AskCard's own (`features/ask/model.ts`), re-exported; `words` and `code` build parts.
 * Depends on: protocol/core types, features/ask/model. Called by: allowed/entries, the Allowed rows.
 */
import type { CoversPart } from "../../../protocol/core";

// One implementation for the card, the grants list and this page.
export { coversCapitalised, coversText } from "../../ask/model";

/** Plain words. */
export function words(text: string): CoversPart {
  return { text, code: false };
}

/** Exact text shown as code. */
export function code(text: string): CoversPart {
  return { text, code: true };
}
