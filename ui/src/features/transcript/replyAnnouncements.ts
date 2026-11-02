/** Announce completed turns once, never snapshots or streaming patches. Uses the shared announcer and preferences. */
import { announce } from "../../state/announcer";
import { finalAnswer } from "../../state/voiceChatTurns";
import { obj, str } from "./model/entries";
import { speakable } from "../voice/speechText";
import type { App } from "../shell/AppContext";

export function startReplyAnnouncements(app: App): () => void {
  const core = app.core;
  if (core === null) return () => undefined;
  const seen = new Set<string>();
  return core.listen((event) => {
    if (event.type !== "change" || event.change.kind !== "turnUpserted") return;
    const { sessionId, turn } = event.change;
    const ended = obj(turn);
    const id = str(ended?.["id"]),
      status = str(ended?.["status"]);
    if (id === null || !["completed", "failed", "interrupted"].includes(status ?? "")) return;
    const key = `${sessionId}:${id}`;
    if (seen.has(key)) return;
    seen.add(key);
    if (seen.size > 1000) {
      const first = seen.values().next().value;
      if (first !== undefined) seen.delete(first);
    }
    if (app.shell.store.get().selected !== sessionId || app.shell.store.get().settings.open) return;
    if (status === "failed") {
      announce("Vibe stopped. The turn failed.", { assertive: true });
      return;
    }
    if (status === "interrupted") {
      announce("Vibe stopped.");
      return;
    }
    const mode = app.accessibility.store.get().reply;
    const plain = speakable(finalAnswer(core.store.get().sessions[sessionId], id), 100_000, true);
    const segments = new Intl.Segmenter(undefined, { granularity: "sentence" }).segment(plain);
    const sentence = segments[Symbol.iterator]().next().value?.segment.trim() ?? "";
    const text = mode === "short" ? "" : mode === "whole" ? plain : sentence;
    announce(text === "" ? "Vibe finished." : `Vibe finished. ${text}`);
  });
}
