/** Person-facing names for shipped Tools entries; depends on vocabulary and explicit origin metadata.
 * Called by Connectors, Marketplace and Skills. Exact config and callable identities remain separate.
 */
import type { ServerView, SkillRow } from "../protocol/core";
import { vocabulary, type Term } from "../vocabulary";

function known(
  entries: Readonly<Record<string, Term>>,
  name: string,
  scope: string | null | undefined,
): Term | null {
  return scope === "builtin" && Object.hasOwn(entries, name) ? (entries[name] ?? null) : null;
}

/** Only the shipped plugin receives its product label; namesakes keep their manifest text. */
export function builtinPluginText(name: string, scope: string): Term | null {
  return known(vocabulary.builtinPlugins, name, scope);
}

/** Ambiguous, personal, project and catalog skills keep their exact names and descriptions. */
export function builtinSkillText(row: Pick<SkillRow, "name" | "source">): Term | null {
  return known(vocabulary.builtinSkills, row.name, row.source);
}

/** A server receives a product label only for the core’s explicit built-in origin. */
export function builtinServerText(server: ServerView): Term | null {
  return server.configured
    ? null
    : known(vocabulary.builtinPlugins, server.plugin ?? server.name, server.source);
}
