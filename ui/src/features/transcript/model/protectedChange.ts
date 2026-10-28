/** Protected review text and unified-diff adapter; used by the transcript card and attention stores. */
import type { JsonValue, ProtectedChangeView, ProtectedFileView } from "../../../protocol/core";
import type { DiffLine, FileDiff } from "./diff";
import { type Obj, obj } from "./entries";

/** Reads app-owned review history entries, tolerating unrelated server history. */
export function protectedChangeOf(entry: Obj): ProtectedChangeView | null {
  if (entry["type"] !== "protected_change") return null;
  const value = obj(entry["proposal"]);
  if (value === null || typeof value["id"] !== "string" || !Array.isArray(value["files"]))
    return null;
  return value as unknown as ProtectedChangeView;
}
/** Pending identities for session attention and notifications. */
export function pendingProtectedChanges(history: JsonValue | undefined): readonly string[] {
  if (!Array.isArray(history)) return [];
  return history.flatMap((entry) => {
    const value = obj(entry);
    const proposal = value === null ? null : protectedChangeOf(value);
    return proposal?.status === "pending" ? [proposal.id] : [];
  });
}

/** The ask ids of the Chrome Control cards in `history` that wait for the user's answer (the core's `open`). */
function openChromeAsks(history: JsonValue | undefined): readonly string[] {
  if (!Array.isArray(history)) return [];
  return history.flatMap((entry) => {
    const card = obj(obj(entry)?.["chromeAsk"]);
    return card?.["open"] === true && typeof card["askId"] === "string" ? [card["askId"]] : [];
  });
}

/** Every ask in `history` that waits for the user: pending protected changes and open Chrome Control cards. */
export function pendingHistoryAsks(history: JsonValue | undefined): readonly string[] {
  return [...pendingProtectedChanges(history), ...openChromeAsks(history)];
}
/** Converts captured unified text to the existing read-only DiffView lines. */
export function protectedDiff(file: ProtectedFileView): FileDiff {
  const lines: DiffLine[] = [];
  let oldNo = 0;
  let newNo = 0;
  let hunk = false;
  for (const raw of (file.diff ?? "").split("\n")) {
    const header = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(raw);
    if (header !== null) {
      if (hunk) lines.push({ kind: "gap", oldNo: null, newNo: null, text: "" });
      hunk = true;
      oldNo = Number(header[1]);
      newNo = Number(header[2]);
    } else if (hunk && raw.startsWith("+")) {
      lines.push({ kind: "add", oldNo: null, newNo: newNo++, text: raw.slice(1) });
    } else if (hunk && raw.startsWith("-")) {
      lines.push({ kind: "del", oldNo: oldNo++, newNo: null, text: raw.slice(1) });
    } else if (hunk && raw.startsWith(" ")) {
      lines.push({ kind: "ctx", oldNo: oldNo++, newNo: newNo++, text: raw.slice(1) });
    } else if (hunk && raw.startsWith("\\")) {
      lines.push({ kind: "ctx", oldNo: null, newNo: null, text: raw });
    }
  }
  return {
    path: file.path,
    lines,
    added: file.addedLines,
    removed: file.removedLines,
    newFile: file.kind === "add",
  };
}
function bytes(n: number): string {
  return n < 1024 ? `${String(n)} B` : `${(n / 1024).toFixed(1)} KB`;
}
/** Specified byte transitions, including binary removals. */
export function protectedSize(file: ProtectedFileView): string {
  if (file.contentKind === "binary")
    return `binary file, ${bytes(file.beforeBytes)} → ${bytes(file.afterBytes)}`;
  if (file.kind === "add") return `new, ${bytes(file.afterBytes)}`;
  if (file.kind === "remove") return `removed, ${bytes(file.beforeBytes)}`;
  return `${bytes(file.beforeBytes)} → ${bytes(file.afterBytes)}`;
}
/** Plain-text lifecycle copy shared by fixtures and the card. */
export function protectedStateText(p: ProtectedChangeView): string {
  const time = new Date(p.decidedAtMs ?? p.createdAtMs).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
  switch (p.status) {
    case "pending":
      return "Waiting for you";
    case "applying":
      return "Applying…";
    case "applied":
      return `Applied at ${time}: ${String(p.files.length)} file(s).${p.files.some((f) => f.path.startsWith("skills/")) ? " New chats use it." : ""}`;
    case "declined":
      return p.stopped
        ? "Declined: the turn was stopped."
        : `Declined at ${time}. Nothing was changed.`;
    case "stale":
      return `Nothing was changed: ${p.problem?.path ?? "a file"} changed after this was proposed. Ask Vibe to propose it again.`;
    case "failed":
      return `Stopped at ${p.problem?.path ?? "the proposal"}: ${p.problem?.message ?? "application failed"}. Applied before that: ${p.applied.join(", ") || "none"}. Not applied: ${
        p.files
          .filter((f) => !p.applied.includes(f.path))
          .map((f) => f.path)
          .join(", ") || "none"
      }.`;
  }
}
