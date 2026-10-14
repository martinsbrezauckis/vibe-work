/**
 * The composer's file picker contract and browser-file adapter. The filter is the union of the former image
 * picker and app/dialogs.rs DOCUMENT_TYPES; the core still decides document kinds from bytes (vw-documents).
 * Depends on: the drop router's item type. Called by: ComposerTools, Composer and the drop router.
 * Invariant: picker, paste and drop share byte-based routing; filenames only describe a refused type.
 */
import type { DroppedItem } from "./drop";

/** The picker offers every extension from the native document picker and the four inline image MIME types. */
export const ATTACH_ACCEPT =
  "image/png,image/jpeg,image/gif,image/webp,.pdf,.docx,.pptx,.xlsx,.csv,.txt,.md,.png,.jpg,.jpeg,.webp,.gif,.tif,.tiff,.bmp,.avif";

/** The single paperclip's tooltip and accessible name. Office names refer to the modern formats only. */
export const ATTACH_LABEL =
  "Attach files (images, PDF, Word, PowerPoint, Excel, CSV, text, Markdown)";

/** Kinds accepted by vw-documents, including the images that take its OCR path. */
const SUPPORTED =
  "PDF, Word (DOCX), PowerPoint (PPTX), Excel (XLSX), CSV, UTF-8 text/Markdown, PNG, JPEG, GIF, WebP, TIFF, BMP, AVIF";

/** Adapts picked or pasted files to the same byte reader as a drop; never reads a filesystem path. */
export function fileItems(files: readonly File[]): DroppedItem[] {
  return files.map((file) => ({
    name: file.name,
    size: file.size,
    folder: false,
    bytes: async () => new Uint8Array(await file.arrayBuffer()),
  }));
}

/** Adds the named type and supported kinds to type refusals, retaining other core messages (including limits). */
export function attachmentRefusal(name: string, reason: string): string {
  const unsupported = reason.startsWith("This file type is not supported.");
  const archive = reason.startsWith("Archives are not unpacked.");
  if (!unsupported && !archive) return reason;
  const extension = /\.([a-z0-9]{1,20})$/i.exec(name)?.[1]?.toUpperCase();
  return `This ${extension ?? "binary"} file is not supported.${archive ? ` ${reason}` : ""} Supported: ${SUPPORTED}.`;
}
