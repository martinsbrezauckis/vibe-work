/**
 * The agent browser's presentation model: pure functions the Browser tab's components and its
 * controller use, so they only render.
 * - Whether a page is the session's own app preview (annotation is for that only).
 * - The address bar's text to a URL (`example.com` -> `https://example.com`).
 * - An annotation as the context chip and the picture the composer sends (always data, never a command).
 * - Words for the action log and the annotation states.
 * Depends on: protocol types, pages/model (`Chip`). Called by: browser.ts, AgentStrip, ActionLog, Annotations,
 * browserRoutes.ts, UI tests.
 * Invariants: every page-derived string (names, text, routes, source hints) is quoted as data in the chip text.
 */
import type { Annotation, LogEntry } from "../../protocol/core";
import type { Chip } from "../pages/model";

/** Who made or last edited a note: the window's own controls, or a Debug API client (`ui/browser`). */
export type NoteOrigin = "ui" | "debug-api";

/** One annotation the user collects before sending. */
export interface NoteItem {
  id: string;
  annotation: Annotation;
  /** The user's note. */
  note: string;
  /**
   * Provenance: "debug-api" once a Debug API request created or edited the note, and it stays so;
   * its chip and crop then carry the Debug API's mark whoever sends them.
   */
  by: NoteOrigin;
  /** draft (not sent yet), sent, then after the agent's turn: changed / unchanged / missing. */
  status: "draft" | "sent" | "changed" | "unchanged" | "missing";
}

/** True for a page of this computer (the session's own app preview): `127.0.0.1`, `localhost`, `[::1]`. */
export function isLocalPage(url: string | null | undefined): boolean {
  if (url === null || url === undefined) return false;
  try {
    const host = new URL(url).hostname;
    return host === "127.0.0.1" || host === "localhost" || host === "[::1]";
  } catch {
    return false;
  }
}

/**
 * The address bar's text as a URL: a missing scheme becomes `http://` for this computer and for private-network
 * IPv4 addresses (a router's page is usually plain http), `https://` otherwise.
 */
export function addressToUrl(text: string): string {
  const value = text.trim();
  if (value === "") return value;
  if (/^[a-z][a-z0-9+.-]*:(?!\d)/i.test(value)) return value;
  if (/^(localhost|127\.0\.0\.1|\[::1\])([:/]|$)/i.test(value)) return `http://${value}`;
  if (/^(10\.\d+|192\.168|172\.(1[6-9]|2\d|3[01]))\.\d+\.\d+([:/]|$)/.test(value))
    return `http://${value}`;
  return `https://${value}`;
}

function clip(text: string, max: number): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}

/** The mark a sent note's chip and crop carry: the Debug API's when the note or the send came from it. */
export function sendMark(item: NoteItem, origin: NoteOrigin): NoteOrigin {
  return item.by === "debug-api" || origin === "debug-api" ? "debug-api" : "ui";
}

/** What the agent receives for one annotation (the composer sends it as a resource block). */
export function annotationChip(item: NoteItem): Chip {
  const a = item.annotation;
  const what = a.name !== "" ? `${a.role} "${clip(a.name, 80)}"` : a.role || a.mode;
  const lines = [
    `Note on the app preview at ${a.url}${a.route !== "" ? ` (route ${a.route})` : ""}:`,
    `"${clip(item.note, 2000) || "(no note)"}"`,
    a.mode === "box"
      ? `Area: a box of ${String(a.rect.width)}x${String(a.rect.height)} px around ${what}`
      : `Element: ${what}`,
  ];
  if (a.reference !== null) {
    lines.push(`Browser ref: ${a.reference} (refs change: call browser_read before acting on it)`);
  }
  if (a.path !== "") lines.push(`DOM path: ${a.path}`);
  if (a.source !== null) lines.push(`Source hint from the page's build: ${a.source}`);
  if (a.text !== "") lines.push(`Text: "${clip(a.text, 500)}"`);
  if (a.cropPngBase64 !== null) lines.push("A crop of it is attached as a picture.");
  return { label: `Preview note · ${clip(what, 40)}`, source: "browser", text: lines.join("\n") };
}

/** Bytes of a base64 string (for the composer's picture limits). */
export function base64Bytes(data: string): number {
  const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
  return Math.floor((data.length * 3) / 4) - padding;
}

/** Who did a logged action, for the log. */
export function actorText(entry: LogEntry): string {
  return entry.by === "agent" ? "Agent" : entry.by === "debugApi" ? "Debug API" : "You";
}

/** The log line's outcome word. */
export function outcomeText(entry: LogEntry): string {
  switch (entry.outcome) {
    case "done":
      return "done";
    case "refused":
      return "refused";
    case "failed":
      return "failed";
    case "stopped":
      return "stopped";
  }
}

/** Words for an annotation's state. */
export function statusText(status: NoteItem["status"]): string {
  switch (status) {
    case "draft":
      return "Not sent yet";
    case "sent":
      return "Sent: waiting for the agent";
    case "changed":
      return "Changed (likely addressed)";
    case "unchanged":
      return "No change yet";
    case "missing":
      return "Not found on the page now";
  }
}
