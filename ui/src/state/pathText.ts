/**
 * Small text helpers the features share: the last folder name of a path, and base64 of bytes.
 * Depends on: nothing. Called by: search results, the chat-copy location line, the explorer panel, the Allowed
 * list, image attachments and dictation.
 * Invariants: `folderName` splits on both separators, ignores empty parts (doubled or trailing separators) and
 * returns the path itself when nothing is left (a root, or an empty path); `null` reads as "". `toBase64` encodes in
 * chunks, because `String.fromCharCode` takes a limited number of arguments.
 */

/** The last folder name of a path (`/work/shop` -> `shop`, `C:\work\shop` -> `shop`, `/` -> `/`). */
export function folderName(path: string | null): string {
  if (path === null) return "";
  return (
    path
      .split(/[\\/]+/)
      .filter((part) => part !== "")
      .at(-1) ?? path
  );
}

/** Base64 of `bytes`. */
export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}
