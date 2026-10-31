/**
 * The Large 4 rows of a model list: the thinking levels a picker offers for one model, from the levels Vibe reports
 * for it, and which Large 4 rows the pickers list.
 * Mistral Large 4 cannot think Off: the key proxy sends Off without a level and Large 4 reasons anyway
 * (`crates/vw-keyproxy/src/reasoning.rs`), while Low, Medium and High go out as picked. Vibe Work's own `large-4`
 * entry lists exactly those three (`crates/vw-host/src/default_model.rs`), and so does Mistral's own Large 4 entry,
 * so both keep the list Vibe reports like every other model. Only a Large 4 row whose report offers Off (an entry a
 * person wrote without levels reports Off and High) or reports nothing is offered High only, and a saved Off or Low
 * shows and runs as High there.
 * While Vibe Work's own `large-4` entry is listed, Mistral's own Large 4 entry (from Mistral's lists: `growthbook` or
 * `catalog`, the row `crates/vw-core-api/src/model_check.rs` fills with facts) is left out of the composer and
 * Settings lists: it is the same model, and its alias comes from the account's list, which can name an id Mistral no
 * longer serves and leaves the list when "Take part in Mistral's experiments" is off. A chat or default already on
 * that alias keeps it: its list shows that entry as its one Large 4 in place of ours (nothing moves), and it takes the
 * usual fallback when the entry is gone.
 * Depends on nothing. Called by: composer/model/models (the composer menu and new-chat preference),
 * modelChoice (Settings, the default-model card).
 * Invariants: pure; an unknown model shape is never treated as Large 4.
 */

/** API names of Mistral Large 4 (the key proxy's Large 4 ids). */
const LARGE_4 = new Set(["mistral-large-4", "mistral-large-4-0"]);
/** The alias of Vibe Work's own Large 4 entry (`crates/vw-host/src/default_model.rs`). */
const OWN_ALIAS = "large-4";

type Row = { readonly [key: string]: unknown } | null | undefined;

const isLarge4 = (row: Row): boolean => {
  const name = row?.["name"];
  return typeof name === "string" && LARGE_4.has(name);
};

/** Mistral's own Large 4 row (from Mistral's lists), never Vibe Work's own entry. */
const isMistrals = (row: Row): boolean =>
  isLarge4(row) &&
  row?.["alias"] !== OWN_ALIAS &&
  (row?.["source"] === "growthbook" || row?.["source"] === "catalog");

/**
 * A filter for a model list (`ModelConfigView` rows) that keeps one Large 4 where Vibe Work's own `large-4` entry is
 * in the same list: ours, or, when `inUse` (the chat's model or the Settings default) is Mistral's own Large 4 entry,
 * that entry in place of ours. True for every other row.
 */
export function listedRows(
  rows: readonly Row[],
  inUse: string | null = null,
): (row: Row) => boolean {
  const ours = rows.some((row) => row?.["alias"] === OWN_ALIAS && isLarge4(row));
  if (!ours) return () => true;
  if (!rows.some((row) => isMistrals(row) && row?.["alias"] === inUse)) {
    return (row) => !isMistrals(row);
  }
  return (row) =>
    isMistrals(row) ? row?.["alias"] === inUse : !(row?.["alias"] === OWN_ALIAS && isLarge4(row));
}

/**
 * True for a Large 4 model row (`ModelConfigView`: `name`, `provider`, `thinkingLevels`) whose report offers Off or
 * no levels: High is its only true choice there.
 */
export function alwaysThinks(
  model: { readonly [key: string]: unknown } | null | undefined,
): boolean {
  const name = model?.["name"];
  const provider = model?.["provider"];
  const levels = model?.["thinkingLevels"];
  return (
    typeof name === "string" &&
    LARGE_4.has(name) &&
    (provider === undefined || provider === "mistral") &&
    (!Array.isArray(levels) || levels.includes("off"))
  );
}

/** The levels a picker offers: High only for a model that always thinks, else the reported list. */
export function offeredLevels(always: boolean, reported: readonly string[]): readonly string[] {
  return always ? ["high"] : reported;
}

/** The level a picker shows and sends: any set level of a model that always thinks is High. */
export function effectiveLevel(always: boolean, level: string | null): string | null {
  return always && level !== null ? "high" : level;
}
