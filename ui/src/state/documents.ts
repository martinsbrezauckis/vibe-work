/**
 * Documents in the composer. Files come only from the native picker (`client.pickDocuments`, Rust opens it)
 * or the composer's shared picker/paste/drop router
 * (the window reads the files' bytes and sends them with `addDropped`; Tauri's own drop handling is off); the
 * webview receives a project path only after `documents/save` has written original bytes. Every file
 * becomes a path context chip. An OCR kind also has an optional Read text chip; small extracted text
 * joins the path in model context, while large text and binary files remain path only.
 * Depends on: coreStore, composer (context chips, images, send guard), shellStore (selected session),
 * toastStore, composer image model, protocol types. Called by: createApp, the document views, the
 * `ui/documents` route.
 * Invariants: the path context may be sent without OCR. Every command a Debug API route triggers carries
 * the Debug API origin (`actions("debug-api")`); an extract from that origin asks the core for native confirmation.
 */
import type { CoreEvent, DocumentView, DocumentsRequest, DocumentsResult } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import { imageFromBytes, isImage } from "../features/composer/model/images";
import type { Composer } from "./composer";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import { NEW_CHAT } from "./newChatKey";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";

/** The document state of the composers. */
export interface DocumentsState {
  /** Per session: documents not yet in the message as text (waiting, reading, failed). */
  chips: Readonly<Record<string, readonly DocumentView[]>>;
  /** Document ids with a UI action in flight. */
  busy: Readonly<Record<string, true>>;
  /** The context chip whose text is shown in the preview dialog. */
  preview: { sessionId: string; contextId: string } | null;
}

const EMPTY: DocumentsState = { chips: {}, busy: {}, preview: null };

/** Stands in for the store when the app has no core (no documents controller): never changes. */
export const NO_DOCUMENTS = createStore<DocumentsState>(EMPTY);

/** Pages in words ("3 pages", "1 page", "pages unknown"). */
export function pagesText(pages: number | null): string {
  if (pages === null) return "pages unknown";
  return `${String(pages)} page${pages === 1 ? "" : "s"}`;
}

/** The sentence a waiting OCR document shows before anything is sent. */
export function quotaText(doc: DocumentView): string {
  return doc.pages === null
    ? "Reading it uses Mistral OCR quota; Mistral counts the pages."
    : `Reading it uses ${pagesText(doc.pages)} of your Mistral OCR quota.`;
}

/** What was kept of a long document, or `null` when nothing was cut. */
export function keptText(kept: DocumentView["kept"]): string | null {
  if (kept === null) return null;
  const unit = kept.unit === "pages" ? "pages" : "lines";
  const part =
    kept.kept === 0
      ? `only the start of the first of ${String(kept.of)} ${unit}`
      : `${unit} 1-${String(kept.kept)} of ${String(kept.of)}`;
  return `Long document: kept ${part} (400,000 characters); the rest is not sent.`;
}

function setIn<T>(map: Readonly<Record<string, T>>, key: string, value: T | undefined) {
  const next: Record<string, T> = { ...map };
  if (value === undefined) Reflect.deleteProperty(next, key);
  else next[key] = value;
  return next;
}

function decode(base64: string): Uint8Array {
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
  return out;
}

/** Creates the documents controller. */
export function createDocuments(deps: {
  core: CoreController;
  composer: Composer;
  shell: Shell;
  toasts: Toasts;
  /** OCR readiness; local attachment operations remain available without a key. */
  canUseKey?: () => boolean;
}) {
  const { core, composer, shell, toasts } = deps;
  const store = createStore<DocumentsState>(EMPTY);
  const saved = new Map<
    string,
    { sessionId: string; path: string; contextId: string; pending: boolean; inFlight: boolean }
  >();
  const replacing = new Set<string>();

  const modelText = (doc: DocumentView, path: string, body: string | null): string => {
    const lead = `Attached file: ${doc.name}\nAbsolute path: ${path}`;
    return (doc.kind === "text" || doc.kind === "csv") &&
      doc.size <= 16 * 1024 &&
      body !== null &&
      new TextEncoder().encode(body).length <= 16 * 1024
      ? `${lead}\n\nExtracted text:\n${body}`
      : `${lead}\nRead the file at this path when its content is needed.`;
  };

  const sessionOf = (id: string): string | null => {
    for (const [sid, list] of Object.entries(store.get().chips)) {
      if (list.some((d) => d.id === id)) return sid;
    }
    return null;
  };
  const putChip = (sessionId: string, doc: DocumentView): void => {
    store.set((s) => {
      const list = s.chips[sessionId] ?? [];
      const next = list.some((d) => d.id === doc.id)
        ? list.map((d) => (d.id === doc.id ? doc : d))
        : [...list, doc];
      return { ...s, chips: setIn(s.chips, sessionId, next) };
    });
  };
  const dropChip = (sessionId: string, id: string): void => {
    store.set((s) => {
      const left = (s.chips[sessionId] ?? []).filter((d) => d.id !== id);
      return { ...s, chips: setIn(s.chips, sessionId, left.length > 0 ? left : undefined) };
    });
  };
  const setBusy = (id: string, on: boolean): void => {
    store.set((s) => ({ ...s, busy: setIn(s.busy, id, on ? (true as const) : undefined) }));
  };

  /** Runs one documents request with the origin of the action set. */
  const request = async (
    origin: "ui" | "debug-api",
    req: DocumentsRequest,
  ): Promise<DocumentsResult> => {
    const result = await core.run(
      { type: "documents", request: req },
      origin === "debug-api" ? { origin } : undefined,
    );
    if (result.type !== "documents") throw new Error("unexpected answer to a documents request");
    return result.result;
  };

  /** Gives the agent the saved absolute path and, for small text, the extracted content. */
  const toContext = async (
    origin: "ui" | "debug-api",
    sessionId: string,
    doc: DocumentView,
    text: string | null,
    path: string,
  ): Promise<void> => {
    let body = text;
    if (body === null && doc.status === "ready") {
      const read = await request(origin, { op: "read", id: doc.id });
      body = read.kind === "document" ? read.text : null;
    }
    const old = saved.get(doc.id);
    if (old !== undefined) {
      replacing.add(doc.id);
      composer.removeContext(old.sessionId, old.contextId);
    }
    const item = composer.addContext(sessionId, {
      label: doc.name,
      source: "document",
      text: body ?? `Original file saved at ${path}. Use Read text for OCR if needed.`,
      modelText: modelText(doc, path, body),
      uri: `vw://doc/${doc.id}`,
      doc: {
        kindLabel: doc.kindLabel,
        pages: doc.pagesRead ?? doc.pages,
        chars: doc.chars ?? body?.length ?? 0,
        kept: doc.kept,
        model: doc.model,
      },
    });
    saved.set(doc.id, {
      sessionId,
      path,
      contextId: item.id,
      pending: doc.status !== "ready",
      inFlight: false,
    });
    replacing.delete(doc.id);
    if (doc.status === "ready") {
      dropChip(sessionId, doc.id);
      await request(origin, { op: "remove", id: doc.id }).catch(() => undefined);
    }
  };

  /** Places documents the user added into `sessionId`'s composer. */
  const place = async (origin: "ui" | "debug-api", sessionId: string, docs: DocumentView[]) => {
    const added: DocumentView[] = [];
    const refused: { name: string; reason: string }[] = [];
    for (const doc of docs) {
      try {
        const result = await request(origin, { op: "save", id: doc.id, sessionId });
        if (result.kind !== "saved") throw new Error("unexpected attachment save result");
        await toContext(origin, sessionId, doc, null, result.path);
        if (doc.status !== "ready") putChip(sessionId, doc);
        added.push(doc);
      } catch (error) {
        refused.push({ name: doc.name, reason: toCallError(error).error.message });
        await request(origin, { op: "remove", id: doc.id }).catch(() => undefined);
      }
    }
    return { added, refused };
  };

  const refusedToast = (refused: readonly { name: string; reason: string }[]): void => {
    if (refused.length === 0) return;
    toasts.push({
      kind: "warn",
      bellKind: "failure",
      text: `Not added: ${refused.map((r) => `${r.name}: ${r.reason}`).join(" · ")}`,
    });
  };

  /** One origin's actions (the window's own controls, or the `ui/documents` route). */
  const actions = (origin: "ui" | "debug-api") => ({
    /** Opens the native picker (Rust) and puts the chosen documents into `sessionId`'s composer. */
    async pick(sessionId: string): Promise<number> {
      try {
        const picked = await core.client.pickDocuments();
        const placed = await place(origin, sessionId, picked.added);
        refusedToast([...picked.refused, ...placed.refused]);
        return placed.added.length;
      } catch (thrown) {
        toasts.push({
          kind: "error",
          bellKind: "failure",
          text: toCallError(thrown).error.message,
        });
        return 0;
      }
    },
    /**
     * Adds files picked, pasted or dropped on `sessionId`'s composer (the window read their bytes; Tauri's own drop is off
     *) and places the chips there. Returns what the core added and refused (the caller reports refusals).
     */
    async addDropped(
      sessionId: string,
      files: readonly { name: string; data: string }[],
    ): Promise<{ added: DocumentView[]; refused: { name: string; reason: string }[] }> {
      if (files.length === 0) return { added: [], refused: [] };
      const result = await request(origin, { op: "addDropped", files: [...files] });
      if (result.kind !== "added") return { added: [], refused: [] };
      const placed = await place(origin, sessionId, result.added);
      return { added: placed.added, refused: [...result.refused, ...placed.refused] };
    },
    /** Reads a waiting (or failed) document with Mistral OCR, then makes it a context chip. */
    async extract(sessionId: string, id: string): Promise<DocumentView | null> {
      if (deps.canUseKey?.() === false) return null;
      setBusy(id, true);
      try {
        const result = await request(origin, { op: "extract", id });
        if (result.kind !== "document") return null;
        if (result.document.status === "ready") {
          const path = saved.get(id)?.path;
          if (path === undefined) throw new Error("attachment path is unavailable");
          await toContext(origin, sessionId, result.document, result.text, path);
        } else putChip(sessionId, result.document);
        return result.document;
      } catch (thrown) {
        if (deps.canUseKey?.() === false) return null;
        const error = toCallError(thrown).error;
        if (error.code !== "confirmationDeclined") composer.note(sessionId, error.message);
        throw toCallError(thrown);
      } finally {
        setBusy(id, false);
      }
    },
    /** Removes a document chip (and the core's copy). */
    async remove(sessionId: string, id: string): Promise<void> {
      const item = saved.get(id);
      saved.delete(id);
      if (item !== undefined) composer.removeContext(item.sessionId, item.contextId);
      dropChip(sessionId, id);
      await request(origin, { op: "remove", id }).catch(() => undefined);
    },
    /** Puts a PNG/JPEG/WebP/GIF document into the message as a picture instead of reading it. */
    async asPicture(sessionId: string, id: string): Promise<boolean> {
      const result = await request(origin, { op: "takePicture", id });
      if (result.kind !== "picture") return false;
      const already = (composer.store.get().images[sessionId] ?? []).length;
      const image = imageFromBytes(
        result.name,
        decode(result.data),
        composer.nextId("img"),
        already,
      );
      if (!isImage(image)) {
        composer.note(sessionId, `Not attached: ${image.name}: ${image.reason}`);
        return false;
      }
      const item = saved.get(id);
      saved.delete(id);
      if (item !== undefined) composer.removeContext(item.sessionId, item.contextId);
      composer.addImage(sessionId, origin === "debug-api" ? { ...image, by: "debug-api" } : image);
      dropChip(sessionId, id);
      await request(origin, { op: "remove", id }).catch(() => undefined);
      return true;
    },
    /** Shows (or with `null` hides) the preview of a document context chip. */
    preview(target: DocumentsState["preview"]): void {
      store.set((s) => ({ ...s, preview: target }));
    },
  });

  /** Saved paths can be sent immediately; OCR is optional. */
  const blocking = (): string | null => null;
  composer.setSendGuard(blocking);

  const release = (id: string, item: { sessionId: string; pending: boolean }): void => {
    saved.delete(id);
    if (item.pending) {
      dropChip(item.sessionId, id);
      void request("ui", { op: "remove", id }).catch(() => undefined);
    }
  };
  composer.setContentTaken((sessionId, context) => {
    for (const item of saved.values())
      if (item.sessionId === sessionId && context.some((chip) => chip.id === item.contextId))
        item.inFlight = true;
  });
  composer.setContentSettled((sessionId, context, sent) => {
    for (const [id, item] of saved) {
      if (item.sessionId !== sessionId || !context.some((chip) => chip.id === item.contextId))
        continue;
      const restored = (composer.store.get().context[sessionId] ?? []).some(
        (chip) => chip.id === item.contextId,
      );
      item.inFlight = false;
      if (sent || !restored) release(id, item);
    }
  });

  // A sent or removed path chip no longer needs the pending OCR copy. The project file stays.
  composer.store.subscribe(() => {
    for (const [id, item] of saved) {
      if (replacing.has(id)) continue;
      if (composer.isTaking(item.sessionId)) continue;
      if (item.inFlight) continue;
      if ((composer.store.get().context[item.sessionId] ?? []).some((c) => c.id === item.contextId))
        continue;
      release(id, item);
    }
  });

  // Drops arrive as events (Rust got the paths); picker results come back from `pick` itself.
  core.listen((event: CoreEvent) => {
    if (event.type === "documentsAdded" && event.via === "drop") {
      const selected = shell.store.get().selected;
      refusedToast(event.refused);
      if (selected === null) {
        composer.note(NEW_CHAT, "Open a session first, then drop the files on it.");
        for (const doc of event.added) {
          void request("ui", { op: "remove", id: doc.id }).catch(() => undefined);
        }
        return;
      }
      void place("ui", selected, event.added).then((placed) => {
        refusedToast(placed.refused);
      });
    } else if (event.type === "documentChanged") {
      const sid = sessionOf(event.document.id);
      if (sid !== null && event.document.status !== "ready") putChip(sid, event.document);
    } else if (event.type === "lagged") {
      // after `lagged` the core's events for this were missed: read again what is shown.
      void request("ui", { op: "list" })
        .then((result) => {
          if (result.kind !== "list") return;
          for (const doc of result.documents) {
            const sid = sessionOf(doc.id);
            if (sid !== null && doc.status !== "ready") putChip(sid, doc);
          }
        })
        .catch(() => undefined);
    }
  });

  // A deleted session's waiting documents go with it (as the composer drops its draft and chips).
  core.onSessionsGone((gone) => {
    for (const sid of gone) {
      for (const doc of store.get().chips[sid] ?? []) {
        dropChip(sid, doc.id);
        void request("ui", { op: "remove", id: doc.id }).catch(() => undefined);
      }
    }
  });

  return { store, blocking, actions: actions("ui"), debugApi: actions("debug-api") } as const;
}

/** The documents controller. */
export type Documents = ReturnType<typeof createDocuments>;
