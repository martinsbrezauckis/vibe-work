/**
 * Images the user attaches (paste, drop, the file picker), checked before they reach the composer
 * The webview only ever holds the bytes of a file the user chose; no path is read or sent.
 * The core's gate checks the same limits again on every turn (`vw-gate` image rule), so these checks
 * are for a quick, friendly message, not the security boundary.
 * Depends on: model/message (`ImageItem`). Called by: state/composerStore (attach), tests.
 * Invariants: an image is accepted only when its first bytes match a PNG, JPEG, GIF or WebP signature
 * (the declared type is not trusted), it is at most `MAX_IMAGE_BYTES`, and a message holds at most
 * `MAX_IMAGES`; the result's `mimeType` is the sniffed one.
 */
import type { ImageItem } from "./message";

/** Largest image accepted, in bytes (the gate uses the same number). */
export const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
/** Most images per message (the gate uses the same number). */
export const MAX_IMAGES = 4;

/** Image types Vibe and the transcript accept (the transcript renders the same four). */
export type ImageType = "image/png" | "image/jpeg" | "image/gif" | "image/webp";

/** The image type the bytes start with, or `null`. */
export function sniffImage(bytes: Uint8Array): ImageType | null {
  const at = (i: number) => bytes[i];
  const starts = (sig: readonly number[], offset = 0) =>
    bytes.length >= offset + sig.length && sig.every((b, i) => at(offset + i) === b);
  if (starts([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return "image/png";
  if (starts([0xff, 0xd8, 0xff])) return "image/jpeg";
  if (starts([0x47, 0x49, 0x46, 0x38])) return "image/gif";
  if (starts([0x52, 0x49, 0x46, 0x46]) && starts([0x57, 0x45, 0x42, 0x50], 8)) return "image/webp";
  return null;
}

/** Base64 of `bytes` (chunked: `String.fromCharCode(...big)` overflows the stack). */
export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/** Why an image was not attached, in words for a toast. */
export type ImageRefusal = { name: string; reason: string };

/**
 * Checks one file's bytes and turns them into an `ImageItem`, or explains why not. `already` is the
 * number of images the message holds.
 */
export function imageFromBytes(
  name: string,
  bytes: Uint8Array,
  id: string,
  already: number,
): ImageItem | ImageRefusal {
  const label = name.trim() === "" ? "image" : name.trim().slice(0, 120);
  if (already >= MAX_IMAGES) {
    return { name: label, reason: `A message holds at most ${String(MAX_IMAGES)} images.` };
  }
  if (bytes.length > MAX_IMAGE_BYTES) {
    return { name: label, reason: "Images can be at most 10 MB." };
  }
  const mimeType = sniffImage(bytes);
  if (mimeType === null) {
    return { name: label, reason: "Only PNG, JPEG, GIF and WebP images can be attached." };
  }
  return { id, name: label, mimeType, data: toBase64(bytes), bytes: bytes.length };
}

/** True when `result` is an accepted image. */
export function isImage(result: ImageItem | ImageRefusal): result is ImageItem {
  return "data" in result;
}

/** Reads a browser `File` (from paste, drop or the picker) into bytes. */
export async function fileBytes(file: Blob): Promise<Uint8Array> {
  return new Uint8Array(await file.arrayBuffer());
}
