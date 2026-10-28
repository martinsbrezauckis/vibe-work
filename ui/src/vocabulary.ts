/**
 * Product vocabulary for every UI surface. Depends on the same JSON compiled by vw-vocabulary;
 * called by composer, Settings, setup and status views. Wire IDs and runtime model names stay intact.
 */
import source from "../../crates/vw-vocabulary/vocabulary.json";

/** Canonical names, descriptions and action tooltips; edit the shared JSON to change wording. */
export const vocabulary = source;
/** Common headings and notices, also available to Rust. */
export const TEXT = vocabulary.texts;

/** One user-facing concept; the ID used by the protocol is kept separate. */
export interface Term {
  name: string;
  description: string;
  tooltip: string;
}

type Group = Exclude<keyof typeof vocabulary, "texts">;

/** Resolves a known value; custom values retain their actual configured name. */
export function term(group: Group, id: string): Term {
  const entries: Readonly<Record<string, Term>> = vocabulary[group];
  const known = Object.hasOwn(entries, id) ? entries[id] : undefined;
  return known ?? { name: id, description: "", tooltip: id };
}

/** Expands named placeholders in vocabulary copy; values are rendered as plain text by React. */
export function wording(template: string, values: Readonly<Record<string, string>>): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) => values[key] ?? match);
}

/** The offered thinking value, including a custom model's reported levels without inventing choices. */
export function thinkingTerm(level: string): Term {
  const known = vocabulary.thinkingLevels as Readonly<Record<string, Term>>;
  const value = Object.hasOwn(known, level) ? known[level] : undefined;
  return (
    value ?? {
      name: level,
      description: TEXT.customThinkingDescription,
      tooltip: wording(TEXT.thinkingUse, { name: level }),
    }
  );
}

/** A model's runtime display name, falling back to its alias; there is no app-owned model list. */
export function modelTerm(alias: string, displayName?: unknown): Term {
  const name = typeof displayName === "string" && displayName !== "" ? displayName : alias;
  return {
    name,
    description: TEXT.modelDescription,
    tooltip: wording(TEXT.modelUse, { name }),
  };
}

/** The selected vault's product name, or a generic name before selection. */
export function vaultTerm(kind: string | null): Term {
  return kind !== null && Object.hasOwn(vocabulary.vaultKinds, kind)
    ? term("vaultKinds", kind)
    : { name: TEXT.vaultGeneric, description: "", tooltip: TEXT.vaultGeneric };
}
