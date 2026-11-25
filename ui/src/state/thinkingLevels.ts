/**
 * The Large 4 rows of a model list: the thinking levels a picker offers for one model, from the levels Vibe reports
 * for it, and which Large 4 rows the pickers list.
 * Mistral Large 4 cannot think Off: the key proxy sends Off without a level and Large 4 reasons anyway
 * (`crates/vw-keyproxy/src/reasoning.rs`), while Low, Medium and High go out as picked. Vibe Work's own `large-4`
 * entry lists exactly those three (`crates/vw-host/src/default_model.rs`), and so does Mistral's own Large 4 entry,
 * so both keep the list Vibe reports like every other model. Only a Large 4 row whose report offers Off (an entry a
 * person wrote without levels reports Off and High) or reports nothing is offered High only, and a saved Off or Low
 * shows and runs as High there.
 * The pickers show Vibe's catalog and person-owned config rows together. The startup migration removes only the
 * app's exact old Large 4 fallback row. An older experiment row is shown for a chat or default already using it.
 * Mistral's catalog may list one model under two ids with one label (`zai-glm-5-3-flash` and its
 * `zai-glm-5-3-flash-vibe-free` twin): catalog rows that share a label are listed once, the one in use, else the first.
 * Vibe's built-in Medium 3.5 row (`BUILTIN_MEDIUM`) is not offered: Mistral's own list leaves that model out, so a picker
 * that showed it before the list loads would offer a model that is gone once the list is there. It stays listed for
 * the chat or default that already is on it.
 * Depends on nothing. Called by: composer/model/models (the composer menu and new-chat preference),
 * modelChoice (Settings, the default-model card).
 * Invariants: pure; an unknown model shape is never treated as Large 4.
 */

/** API names of Mistral Large 4 (the key proxy's Large 4 ids). */
const LARGE_4 = new Set(["mistral-large-4", "mistral-large-4-0"]);
/** The alias of Vibe Work's own Large 4 entry (`crates/vw-host/src/default_model.rs`). */
const OWN_ALIAS = "large-4";

/**
 * The built-in row of Vibe 3.1.4 for Mistral Medium 3.5 (`MEDIUM_3_5_MODEL_CONFIG`, shown with `source: "local"`).
 * A row that only shares the alias (another name or source, e.g. a project's own model) is not this row.
 */
const BUILTIN_MEDIUM = {
  alias: "mistral-medium-3.5",
  name: "mistral-vibe-cli-latest",
  source: "local",
};

type Row = { readonly [key: string]: unknown } | null | undefined;

const isLarge4 = (row: Row): boolean => {
  const name = row?.["name"];
  return typeof name === "string" && LARGE_4.has(name);
};

const isBuiltinMedium = (row: Row): boolean =>
  Object.entries(BUILTIN_MEDIUM).every(([key, value]) => row?.[key] === value);

/** Mistral's own Large 4 row (from Mistral's lists), never Vibe Work's own entry. */
const isMistrals = (row: Row): boolean =>
  isLarge4(row) &&
  row?.["alias"] !== OWN_ALIAS &&
  (row?.["source"] === "growthbook" || row?.["source"] === "catalog");

/**
 * A filter for older experiment model lists. Catalog-backed views use Vibe's rows without hiding a person's
 * configured model. For older experiment rows, an in-use row takes precedence over the local fallback.
 */
export function listedRows(
  rows: readonly Row[],
  inUse: string | null = null,
): (row: Row) => boolean {
  const twin = catalogTwins(rows, inUse);
  const large4 = large4Rows(rows, inUse);
  return (row) =>
    !twin.has(row) && large4(row) && !(isBuiltinMedium(row) && row?.["alias"] !== inUse);
}

function large4Rows(rows: readonly Row[], inUse: string | null): (row: Row) => boolean {
  const ours = rows.some((row) => row?.["alias"] === OWN_ALIAS && isLarge4(row));
  if (!ours) return () => true;
  if (rows.some((row) => row?.["source"] === "catalog")) return () => true;
  if (!rows.some((row) => isMistrals(row) && row?.["alias"] === inUse)) {
    return (row) => !isMistrals(row);
  }
  return (row) =>
    isMistrals(row) ? row?.["alias"] === inUse : !(row?.["alias"] === OWN_ALIAS && isLarge4(row));
}

/**
 * The catalog rows a picker leaves out because another catalog row has the same label: of each such group the row
 * in use stays, else the first in Vibe's order. Rows of other sources (a person's own) are never left out here.
 */
function catalogTwins(rows: readonly Row[], inUse: string | null): ReadonlySet<Row> {
  const groups = new Map<string, Row[]>();
  for (const row of rows) {
    const label = row?.["displayName"];
    if (row?.["source"] !== "catalog" || typeof label !== "string" || label.trim() === "") continue;
    groups.set(label.trim(), [...(groups.get(label.trim()) ?? []), row]);
  }
  const left = new Set<Row>();
  for (const group of groups.values()) {
    const kept = group.find((row) => row?.["alias"] === inUse) ?? group[0];
    for (const row of group) if (row !== kept) left.add(row);
  }
  return left;
}

/**
 * True when Vibe's warning that the retired `large-4` alias "is not available" needs no notice: the list has no row
 * of that alias and exactly one catalog Large 4 (not a preview), the model Core puts such a chat back on without a
 * notice (`crates/vw-core-api/src/model_choices/retired.rs`). Core alerts on its own when that does not work.
 */
export function retiredAliasSettled(alias: string, rows: readonly Row[]): boolean {
  if (alias !== OWN_ALIAS || rows.some((row) => row?.["alias"] === OWN_ALIAS)) return false;
  const catalog = rows.filter((row) => {
    const label = row?.["displayName"];
    const id = row?.["alias"];
    return (
      row?.["source"] === "catalog" &&
      isLarge4(row) &&
      typeof id === "string" &&
      id !== "" &&
      !(typeof label === "string" && label.toLowerCase().includes("preview"))
    );
  });
  return catalog.length === 1;
}

/**
 * True for a non-catalog Large 4 model row (`ModelConfigView`: `name`, `provider`, `thinkingLevels`) whose report
 * offers Off or no levels: High is its only true choice there. Catalog rows use their reported levels.
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
    model?.["source"] !== "catalog" &&
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
