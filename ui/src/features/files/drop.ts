/**
 * Files attached to a composer. Tauri's own drop handling is off (`dragDropEnabled: false`), so
 * HTML handles drops; the picker and paste use the same router. This decides per file what Vibe will receive, and says so for every
 * file that is not attached (no silent chip, no misleading "only images" message for a PDF).
 *
 * Routing (sizes checked BEFORE reading the bytes):
 * - a folder -> refused ("drop the files inside it");
 * - over 50 MB -> refused (the documents limit);
 * - PNG, JPEG, GIF or WebP (by their bytes) up to 10 MB -> an inline picture (the gate checks it again);
 * - everything else (and bigger pictures) -> a document (`documents addDropped`): PDF/pictures are read automatically
 *   with OCR; Word/PowerPoint/Excel/CSV/UTF-8 text are read locally; archives and binary files
 *   come back refused with the core's sentence.
 *
 * Depends on: composer image model (sniffing, base64), attachments (refusal text). Called by: Composer, the `ui/files drop` route, tests.
 */
import { toCallError } from "../../protocol/client/types";
import { MAX_IMAGE_BYTES, sniffImage, toBase64 } from "../composer/model/images";
import { attachmentRefusal } from "./attachments";

/** The largest file the documents path takes (the core checks it again). */
const MAX_DROP_BYTES = 50 * 1024 * 1024;

/** One dropped item, independent of the browser API it came from. */
export interface DroppedItem {
  name: string;
  size: number;
  folder: boolean;
  /** The file's bytes (read only when the item is attached). */
  bytes: () => Promise<Uint8Array>;
}

/** What a drop did, per file. */
export interface DropReport {
  /** Names attached as pictures. */
  pictures: string[];
  /** Names added as documents (chips). */
  documents: string[];
  /** Names not attached, with the reason. */
  refused: { name: string; reason: string }[];
}

/** What the router needs from the app. */
export interface DropTargets {
  /** Attaches pictures to the composer (the file-drop path); resolves to how many were attached. */
  attachPictures: (files: File[]) => Promise<number>;
  /** Sends files to the documents store; resolves to what it added and refused. */
  addDocuments: (
    files: { name: string; data: string }[],
  ) => Promise<{ added: { name: string }[]; refused: { name: string; reason: string }[] }>;
}

/** The items of an HTML drop (`DataTransfer`), folders recognised through `webkitGetAsEntry`. */
export function droppedItems(transfer: DataTransfer): DroppedItem[] {
  const entries = [...transfer.items].map((item) =>
    typeof item.webkitGetAsEntry === "function" ? item.webkitGetAsEntry() : null,
  );
  return [...transfer.files].map((file, i) => ({
    name: file.name,
    size: file.size,
    folder: entries[i]?.isDirectory === true,
    bytes: async () => new Uint8Array(await file.arrayBuffer()),
  }));
}

/** Routes and attaches `items`; returns the report (the caller shows the refusals). */
export async function dropFiles(
  items: readonly DroppedItem[],
  targets: DropTargets,
): Promise<DropReport> {
  const report: DropReport = { pictures: [], documents: [], refused: [] };
  const pictures: File[] = [];
  const documents: { name: string; data: string }[] = [];
  for (const item of items) {
    if (item.folder) {
      report.refused.push({
        name: item.name,
        reason:
          "Folders cannot be attached. Drop the files inside it, or ask Vibe to look at the folder.",
      });
      continue;
    }
    if (item.size > MAX_DROP_BYTES) {
      report.refused.push({
        name: item.name,
        reason: "This file is larger than 50 MB, the most Vibe Work attaches.",
      });
      continue;
    }
    let bytes: Uint8Array;
    try {
      bytes = await item.bytes();
    } catch {
      report.refused.push({ name: item.name, reason: "The file could not be read." });
      continue;
    }
    if (sniffImage(bytes) !== null && bytes.length <= MAX_IMAGE_BYTES) {
      pictures.push(new File([bytes.slice().buffer], item.name));
    } else {
      documents.push({ name: item.name, data: toBase64(bytes) });
    }
  }
  if (pictures.length > 0) {
    const attached = await targets.attachPictures(pictures);
    report.pictures = pictures.slice(0, attached).map((f) => f.name);
    // Type and size passed above, so the only refusal left is the per-message picture limit (in order).
    for (const file of pictures.slice(attached)) {
      report.refused.push({ name: file.name, reason: "A message holds at most 4 pictures." });
    }
  }
  if (documents.length > 0) {
    try {
      const result = await targets.addDocuments(documents);
      report.documents = result.added.map((d) => d.name);
      report.refused.push(
        ...result.refused.map((r) => ({ ...r, reason: attachmentRefusal(r.name, r.reason) })),
      );
    } catch (thrown) {
      // The core also bounds a single addDropped batch (32 files); report that rejection rather than losing it.
      const reason = toCallError(thrown).error.message;
      report.refused.push(...documents.map((d) => ({ name: d.name, reason })));
    }
  }
  return report;
}

/** The sentence for the refusals of a drop, or `null` when everything was attached. */
export function refusalText(report: DropReport): string | null {
  if (report.refused.length === 0) return null;
  return `Not attached: ${report.refused.map((r) => `${r.name}: ${r.reason}`).join(" · ")}`;
}

/** The app parts a drop uses (structural, so tests can pass fakes). */
export interface DropApp {
  composer: {
    actions: { attachFiles: (sessionId: string, files: readonly File[]) => Promise<number> } | null;
    debugApi: {
      attachFiles: (sessionId: string, files: readonly File[]) => Promise<number>;
    } | null;
    note: (sessionId: string, text: string | null) => void;
  };
  documents: {
    actions: DocumentsDrop;
    debugApi: DocumentsDrop;
  } | null;
  files: { noteDrop: (sessionId: string, report: DropReport) => void } | null;
}

type DocumentsDrop = {
  addDropped: (
    sessionId: string,
    files: readonly { name: string; data: string }[],
  ) => Promise<{ added: { name: string }[]; refused: { name: string; reason: string }[] }>;
};

/**
 * A drop on `sessionId`'s composer, from the window (`ui`) or the `ui/files drop` route (`debug-api`): routes the
 * files, writes the refusals into the composer's note line (stays until the next note) and keeps the report.
 */
export async function runDrop(
  app: DropApp,
  sessionId: string,
  items: readonly DroppedItem[],
  origin: "ui" | "debug-api",
): Promise<DropReport> {
  const pictures = origin === "ui" ? app.composer.actions : app.composer.debugApi;
  const documents =
    app.documents === null
      ? null
      : origin === "ui"
        ? app.documents.actions
        : app.documents.debugApi;
  const noDocuments = "Documents are not available in this build.";
  const report = await dropFiles(items, {
    attachPictures: (files) => pictures?.attachFiles(sessionId, files) ?? Promise.resolve(0),
    addDocuments: (files) =>
      documents?.addDropped(sessionId, files) ??
      Promise.resolve({
        added: [],
        refused: files.map((f) => ({
          name: f.name,
          reason: noDocuments,
        })),
      }),
  });
  app.composer.note(sessionId, refusalText(report));
  app.files?.noteDrop(sessionId, report);
  return report;
}
