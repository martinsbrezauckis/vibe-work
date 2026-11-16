/**
 * One shared folder-name and base64 helper (state/pathText), and no feature keeps its own copy.
 *
 * PREDICT: before the shared helpers, four files defined `folderName` (search results, the chat-copy location line,
 * the explorer panel, the Allowed list) and two defined `toBase64` (image attachments, dictation), so the guard
 * lists six definitions and fails; after the migration it passes. The cases pin the answers every former copy gave
 * (they agreed on each of them), including doubled and trailing separators, roots and the empty path.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import { folderName, toBase64 } from "../../../ui/src/state/pathText";

const SRC = join(__dirname, "../../../ui/src");

describe("shared path and encoding helpers", () => {
  it("folderName gives the last folder name", () => {
    const cases: [string | null, string][] = [
      ["/work/shop", "shop"],
      ["C:\\work\\shop", "shop"],
      ["/work/shop/", "shop"],
      ["a//b", "b"],
      ["C:\\", "C:"],
      ["/", "/"],
      ["", ""],
      [null, ""],
      ["shop", "shop"],
    ];
    for (const [path, name] of cases)
      expect(folderName(path), String(path)).toBe(name);
  });

  it("toBase64 encodes small and chunk-sized inputs", () => {
    expect(toBase64(new Uint8Array([]))).toBe("");
    expect(toBase64(new TextEncoder().encode("Vibe"))).toBe("VmliZQ==");
    const big = new Uint8Array(0x8000 * 2 + 3).map((_, i) => i % 251);
    expect(atob(toBase64(big)).length).toBe(big.length);
  });

  it("no other module of ui/src defines its own copy", () => {
    const copies: string[] = [];
    const walk = (dir: string): void => {
      for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) walk(path);
        else if (
          /\.tsx?$/.test(name) &&
          !path.endsWith(join("state", "pathText.ts")) &&
          // The chat list's chip text, not a path split: "No folder", the temporary-chat name, or the last name.
          !path.endsWith(join("sessions", "listModel.ts")) &&
          /function (folderName|toBase64)\(/.test(readFileSync(path, "utf8"))
        ) {
          copies.push(relative(SRC, path));
        }
      }
    };
    walk(SRC);
    expect(copies).toEqual([]);
  });
});
