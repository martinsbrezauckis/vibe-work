/**
 * The session's "Allow for this session" answers as its header shows them: which grants belong
 * to a session (its own and its subagents'), each grant's words (the covers words, first letter upper-case),
 * its kind and "allowed at 14:02 by you". Revoke is the transcript store's `revokeGrant`.
 * Depends on: protocol/core types, model.ts. Called by: GrantsChip, the `ui/transcript` route.
 * Invariant: the list is the core's (`askGrants`); the window never keeps a grant of its own.
 */
import type { JsonValue, SessionGrant } from "../../protocol/core";
import { toolText } from "../transcript/model/toolLabels";
import { coversCapitalised } from "./model";

/** The subagent session ids of `childSessions` rows (a subagent's grant belongs to its parent's list). */
export function childIds(rows: JsonValue | undefined): readonly string[] {
  if (!Array.isArray(rows)) return [];
  return rows.flatMap((row) => {
    if (row === null || typeof row !== "object" || Array.isArray(row)) return [];
    const id = row["id"] ?? row["sessionId"];
    return typeof id === "string" ? [id] : [];
  });
}

/** A subagent's name from the parent's `childSessions` rows, when Vibe gives one. */
export function childName(rows: JsonValue | undefined, childId: string): string | null {
  if (!Array.isArray(rows)) return null;
  for (const row of rows) {
    if (row === null || typeof row !== "object" || Array.isArray(row)) continue;
    if (row["id"] !== childId && row["sessionId"] !== childId) continue;
    for (const key of ["agentName", "name", "title"]) {
      const value = row[key];
      if (typeof value === "string" && value !== "") return value;
    }
  }
  return null;
}

/** The grants of `sessionId` and of its subagents, oldest first. */
export function grantsOf(
  grants: readonly SessionGrant[],
  sessionId: string,
  children: readonly string[],
): SessionGrant[] {
  return grants.filter((g) => g.session === sessionId || children.includes(g.session));
}

/** The chip's words: "1 allowed for this session" / "3 allowed for this session". */
export function chipText(count: number): string {
  return `${String(count)} allowed for this session`;
}

/** What kind of call a grant answers, in words (the row's `.d` line). */
function grantKind(grant: SessionGrant): string {
  const tool = grant.tool;
  if (tool === "mcp_vw.process_start") return "Background command";
  if (tool === "mcp_vw.secret_run") return "Command with secrets";
  if (grant.permissions.some((p) => p.scope === "outside_directory")) return "Outside the folder";
  if (grant.permissions.some((p) => p.scope === "command_pattern")) return "Shell command";
  if (grant.permissions.some((p) => p.scope === "url_pattern")) return "Web address";
  if (grant.permissions.some((p) => p.scope === "file_pattern")) return "Files";
  if (tool.startsWith("mcp_")) return "MCP tool";
  return "Tool";
}

/** "14:02" in local time. */
function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** The row's second line: kind · allowed at {time} by you (· subagent “{name}”). */
export function grantDetail(grant: SessionGrant, subagent: string | null): string {
  const by = grant.by === "debugApi" ? "via the Debug API" : "by you";
  return [
    grantKind(grant),
    `allowed at ${clock(grant.createdAt)} ${by}`,
    ...(subagent === null ? [] : [`subagent “${subagent}”`]),
  ].join(" · ");
}

/** The row's words. */
export function grantWords(grant: SessionGrant): ReturnType<typeof coversCapitalised> {
  const covers = coversCapitalised(grant.covers);
  return covers.length > 0
    ? covers
    : [{ text: toolText(grant.tool, grant.labels.join(", ") || grant.tool), code: false }];
}
