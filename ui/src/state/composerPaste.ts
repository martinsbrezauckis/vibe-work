/** Text paste insertion shared by the message box and owner route. Depends on the composer store and paste model; retains the writer's origin. */
import type { ComposerStore } from "./composerStore";
import {
  foldsIntoChip,
  nextPasteNumber,
  normalizePaste,
  pasteMarker,
} from "../features/composer/model/pastes";

/** Inserts normalized text or a long-paste marker at a validated draft range. */
export function pasteIntoDraft(
  composer: ComposerStore,
  sessionId: string,
  text: string,
  start: number,
  end: number,
  by: "user" | "debug-api",
): { before: string; after: string; caret: number } {
  const state = composer.store.get();
  const before = state.drafts[sessionId] ?? "";
  let inserted = normalizePaste(text);
  if (foldsIntoChip(inserted)) {
    const item = {
      id: composer.nextId("paste"),
      n: nextPasteNumber(state.pastes[sessionId] ?? []),
      text: inserted,
    };
    composer.addPaste(sessionId, item);
    inserted = pasteMarker(item);
  }
  const after = before.slice(0, start) + inserted + before.slice(end);
  if (by === "debug-api") composer.setDraftFromDebugApi(sessionId, after);
  else composer.setDraft(sessionId, after);
  return { before, after, caret: start + inserted.length };
}
