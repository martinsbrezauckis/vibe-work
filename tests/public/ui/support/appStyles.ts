/** Loads the same local stylesheet graph as the app for jsdom visibility checks; geometry stays in browser tests. */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const source = resolve(import.meta.dirname, "../../../../ui/src");
function stylesheet(path: string): string {
  return readFileSync(path, "utf8").replace(
    /@import "(\.[^"]+)";/g,
    (_all: string, relative: string) =>
      stylesheet(resolve(dirname(path), relative)),
  );
}
const css = [
  ...readFileSync(resolve(source, "main.tsx"), "utf8").matchAll(
    /import "\.\/(styles\/[^"]+\.css)"/g,
  ),
]
  .map((match) => stylesheet(resolve(source, match[1] ?? "")))
  .join("\n");
if (css.length === 0)
  throw new Error("app stylesheet entry points were not found");

/** Reinstalls the real styles if a fixture replaced the document between tests. */
export function installAppStyles(): void {
  if (document.getElementById("app-test-styles") !== null) return;
  const style = document.createElement("style");
  style.id = "app-test-styles";
  style.textContent = css;
  document.head.append(style);
}
