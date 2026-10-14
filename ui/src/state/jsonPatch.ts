/**
 * JSON Patch (RFC 6902) with Vibe's extra `append` operation, applied to a copy of the document.
 * Mirrors `crates/vw-session/src/patch.rs`: ops `add`, `replace`, `remove`, `test` and `append`
 * (string suffix, `vibe/app_server/_patch.py`); `move` and `copy` are not on the Vibe wire and are
 * refused as unknown ops.
 * Depends on: nothing. Called by: `sessionChanges.ts` (session and entry patches).
 * Invariants: atomic (the input is never modified; on any failure the caller keeps the old
 * document); path errors name the pointer, never the values.
 */
import type { JsonValue } from "../protocol/core";

/** A patch could not be applied. */
export class PatchError extends Error {}

type Container = Record<string, JsonValue> | JsonValue[];

function tokens(path: string): string[] {
  if (path === "") return [];
  if (!path.startsWith("/")) throw new PatchError(`bad pointer ${JSON.stringify(path)}`);
  return path
    .slice(1)
    .split("/")
    .map((t) => t.replaceAll("~1", "/").replaceAll("~0", "~"));
}

function isContainer(value: JsonValue | undefined): value is Container {
  return typeof value === "object" && value !== null;
}

function index(token: string, length: number, allowEnd: boolean): number {
  if (allowEnd && token === "-") return length;
  if (!/^(0|[1-9]\d*)$/.test(token)) throw new PatchError(`bad array index ${token}`);
  const n = Number(token);
  if (n > length || (!allowEnd && n === length))
    throw new PatchError(`index ${token} out of range`);
  return n;
}

/** Walks to the parent of the last token. */
function parentOf(doc: JsonValue, path: string): [Container, string] {
  const parts = tokens(path);
  const last = parts.pop();
  if (last === undefined) throw new PatchError("the root cannot be patched this way");
  let node: JsonValue | undefined = doc;
  for (const part of parts) {
    if (!isContainer(node)) throw new PatchError(`path ${path} cannot be resolved`);
    node = Array.isArray(node) ? node[index(part, node.length, false)] : node[part];
  }
  if (!isContainer(node)) throw new PatchError(`path ${path} cannot be resolved`);
  return [node, last];
}

function read(doc: JsonValue, path: string): JsonValue {
  if (path === "") return doc;
  const [parent, key] = parentOf(doc, path);
  const value = Array.isArray(parent) ? parent[index(key, parent.length, false)] : parent[key];
  if (value === undefined) throw new PatchError(`path ${path} cannot be resolved`);
  return value;
}

function same(a: JsonValue, b: JsonValue): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function applyOne(doc: JsonValue, op: Record<string, JsonValue | undefined>): JsonValue {
  const kind = op["op"];
  const path = op["path"];
  if (typeof kind !== "string" || typeof path !== "string")
    throw new PatchError("malformed operation");
  const value = op["value"] ?? null;
  if (kind === "test") {
    if (!same(read(doc, path), value)) throw new PatchError(`test failed at ${path}`);
    return doc;
  }
  if (kind === "append") {
    const current = read(doc, path);
    if (typeof current !== "string" || typeof value !== "string")
      throw new PatchError(`append at ${path} needs strings`);
    return setAt(doc, path, current + value, "replace");
  }
  if (kind === "add" || kind === "replace") return setAt(doc, path, value, kind);
  if (kind === "remove") return removeAt(doc, path);
  throw new PatchError(`unknown patch op ${kind}`);
}

function setAt(doc: JsonValue, path: string, value: JsonValue, mode: "add" | "replace"): JsonValue {
  if (path === "") return value;
  const [parent, key] = parentOf(doc, path);
  if (Array.isArray(parent)) {
    const i = index(key, parent.length, mode === "add");
    if (mode === "add") parent.splice(i, 0, value);
    else parent[i] = value;
  } else {
    if (mode === "replace" && !(key in parent))
      throw new PatchError(`path ${path} cannot be resolved`);
    parent[key] = value;
  }
  return doc;
}

function removeAt(doc: JsonValue, path: string): JsonValue {
  const [parent, key] = parentOf(doc, path);
  if (Array.isArray(parent)) parent.splice(index(key, parent.length, false), 1);
  else if (key in parent) Reflect.deleteProperty(parent, key);
  else throw new PatchError(`path ${path} cannot be resolved`);
  return doc;
}

/**
 * Applies `ops` to a copy of `doc` and returns the copy.
 * @throws PatchError when any operation fails; `doc` is untouched either way.
 */
export function applyPatch(doc: JsonValue, ops: readonly JsonValue[]): JsonValue {
  let working: JsonValue = structuredClone(doc);
  for (const op of ops) {
    if (typeof op !== "object" || op === null || Array.isArray(op))
      throw new PatchError("malformed operation");
    working = applyOne(working, op);
  }
  return working;
}
