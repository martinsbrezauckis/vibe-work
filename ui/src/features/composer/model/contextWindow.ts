/**
 * Context labels for the composer model selector. Depends on projected model fields; called by
 * models.ts. Known API models use Mistral's published window, never a compaction threshold.
 * Other models show only a valid configured count. This changes display, never Vibe's config.
 * The 2.26.1 projection omits provider; exact names identify published model capacity, not endpoint caps.
 */
import { type Obj } from "../../transcript/model/entries";

// Mistral model cards. Keep their units: "1M" does not specify an exact integer.
// https://docs.mistral.ai/models/mistral-large
// https://docs.mistral.ai/models/mistral-medium-3-5-26-04
// https://docs.mistral.ai/models/zai-glm-5-3
const PUBLISHED_WINDOWS: ReadonlyMap<string, string> = new Map([
  ["mistral-large-4", "1M"],
  ["mistral-large-4-0", "1M"],
  ["mistral-medium-3-5", "256k"],
  ["mistral-medium-3.5", "256k"],
  ["zai-glm-5-3", "1M"],
]);

/**
 * One labelled full window. API identity takes precedence over a local alias; the moving Vibe CLI
 * name is recognized only with its pinned Medium 3.5 alias (refresh when the pin's target changes).
 * An explicit non-Mistral provider uses its configured count. Missing/invalid custom counts stay unknown.
 */
export function contextWindowLabel(model: Obj): string {
  const name = model["name"];
  const provider = model["provider"];
  const published =
    provider !== undefined && provider !== "mistral"
      ? undefined
      : name === "mistral-vibe-cli-latest" && model["alias"] === "mistral-medium-3.5"
        ? "256k"
        : typeof name === "string"
          ? PUBLISHED_WINDOWS.get(name)
          : undefined;
  if (published !== undefined) return `Context: ${published} tokens`;
  const count = model["maxContextLength"];
  return typeof count === "number" && Number.isSafeInteger(count) && count > 0
    ? `Context: ${count.toLocaleString("en-US")} tokens`
    : "Context: not reported";
}
