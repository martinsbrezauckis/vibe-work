/**
 * Reads the selected chat's last model-call breakdown through the core command client.
 * Depends on generated Vibe types and coreStore; used by the context popover.
 * Closed panels do no work; refreshed and disposed panels cannot display an older request's result.
 */
import type { ContextBreakdown, ContextCategory, ContextCategoryId } from "../protocol/generated";
import type { JsonValue } from "../protocol/core";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";

export const CONTEXT_LABELS: Record<ContextCategoryId, string> = {
  system: "System",
  tools: "Tools",
  skills: "Skills",
  conversation: "Conversation",
};

function object(value: JsonValue | undefined) {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value : null;
}
function count(value: JsonValue | undefined): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

/** Validate wire data before formatting counts; missing categories are unknown, never zero-filled. */
export function parseContextComposition(result: JsonValue): ContextBreakdown | null {
  const value = object(object(result)?.["breakdown"]);
  if (!value) return null;
  const { contextWindow, compactionThreshold, usedTokens, measured, totalCharacters } = value;
  if (
    !count(contextWindow) ||
    !count(compactionThreshold) ||
    !count(usedTokens) ||
    !count(totalCharacters) ||
    typeof measured !== "boolean"
  )
    return null;
  const raw = value["categories"] ?? [];
  if (!Array.isArray(raw)) return null;
  const categories: ContextCategory[] = [];
  for (const item of raw) {
    const row = object(item);
    const id = row?.["id"];
    if (
      (id !== "system" && id !== "tools" && id !== "skills" && id !== "conversation") ||
      !count(row?.["characters"]) ||
      !count(row["estimatedTokens"]) ||
      categories.some((category) => category.id === id)
    )
      return null;
    categories.push({ id, characters: row["characters"], estimatedTokens: row["estimatedTokens"] });
  }
  if (
    categories.length > 0 &&
    categories.reduce((sum, row) => sum + row.estimatedTokens, 0) !== usedTokens
  )
    return null;
  return {
    contextWindow,
    compactionThreshold,
    usedTokens,
    measured,
    totalCharacters,
    categories: measured ? categories : [],
  };
}

/** Share of the full model window, with small nonzero shares kept visible. */
export function contextShare(tokens: number, window: number): string {
  if (window === 0) return "—";
  const share = (tokens / window) * 100;
  return share > 0 && share < 0.1
    ? "<0.1%"
    : `${share.toLocaleString("en-US", { maximumFractionDigits: 1 })}%`;
}

type State =
  | { status: "loading" | "unavailable"; breakdown: null }
  | { status: "ready"; breakdown: ContextBreakdown };

/** One mounted panel's controller; coalesces usage updates and drops results after disposal. */
export function createContextComposition(core: CoreController, sessionId: string) {
  const store = createStore<State>({ status: "loading", breakdown: null });
  let active = false;
  let running = false;
  let revision = 0;
  const signature = () => {
    const state = core.store.get();
    const chat = state.sessions[sessionId];
    return JSON.stringify([
      state.server.phase,
      chat?.session["contextUsage"],
      chat?.session["contextFill"],
      chat?.session["tokenUsage"],
      chat?.runtime,
    ]);
  };
  const isActive = () => active;
  const read = async () => {
    if (running || !isActive()) return;
    running = true;
    const requested = revision;
    try {
      const result = await core.run({
        type: "call",
        method: "session/context/read",
        params: { sessionId },
      });
      const breakdown = result.type === "reply" ? parseContextComposition(result.result) : null;
      if (active && revision === requested)
        store.set(
          breakdown ? { status: "ready", breakdown } : { status: "unavailable", breakdown: null },
        );
    } catch {
      if (active && revision === requested) store.set({ status: "unavailable", breakdown: null });
    } finally {
      running = false;
      if (active && revision !== requested) void read();
    }
  };
  const refresh = () => {
    revision += 1;
    store.set({ status: "loading", breakdown: null });
    void read();
  };
  return {
    store,
    refresh,
    start() {
      active = true;
      let previous = signature();
      const unsubscribe = core.store.subscribe(() => {
        const next = signature();
        if (next === previous) return;
        previous = next;
        refresh();
      });
      refresh();
      return () => {
        active = false;
        revision += 1;
        unsubscribe();
      };
    },
  };
}
