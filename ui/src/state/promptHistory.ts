/** Prompt recall state and caret decisions; composer wiring calls it before ordinary key handling. */
import type { KeyInfo } from "../features/composer/model/keys";
import type { CoreController } from "./coreStore";
import type { ComposerStore } from "./composerStore";

/** A navigation snapshot, reset by any edit; caret movement within a prompt keeps the snapshot. */
export class PromptHistory {
  private entries: readonly string[] = [];
  private index: number | null = null;
  private recalled = "";

  replace(entries: readonly string[]): void {
    if (this.entries.length === entries.length && this.entries.every((e, i) => e === entries[i]))
      return;
    this.entries = entries;
    this.edit();
  }
  edit(): void {
    this.index = null;
    this.recalled = "";
  }
  key(key: KeyInfo, text: string, start: number, end: number): string | null {
    if (
      key.shiftKey ||
      key.ctrlKey ||
      key.altKey ||
      key.metaKey ||
      key.isComposing ||
      start !== end
    )
      return null;
    if (key.key !== "ArrowUp" && key.key !== "ArrowDown") return null;
    if (this.index !== null && text !== this.recalled) this.edit();
    if (this.index === null && (text !== "" || key.key !== "ArrowUp")) return null;
    const up = key.key === "ArrowUp";
    if (up ? text.slice(0, start).includes("\n") : text.slice(end).includes("\n")) return null;
    const next = (this.index ?? this.entries.length) + (up ? -1 : 1);
    if (next < 0 || next > this.entries.length) return null;
    if (next === this.entries.length) {
      this.edit();
      return "";
    }
    const entry = this.entries[next];
    if (entry === undefined) return null;
    this.index = next;
    this.recalled = entry;
    return entry;
  }
}

/** Refreshes from the CLI file or records an accepted submission; optional recall never blocks sending. */
export async function syncPromptHistory(
  core: CoreController,
  composer: ComposerStore,
  text?: string,
): Promise<void> {
  try {
    const answer = await core.run({
      type: "promptHistory",
      ...(text === undefined ? {} : { text }),
    });
    if (answer.type === "promptHistory") composer.history.set(() => answer.entries);
  } catch {
    /* A missing/unreadable CLI history leaves the current recall list available. */
  }
}
