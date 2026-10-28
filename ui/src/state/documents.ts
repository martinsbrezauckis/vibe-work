/**
 * Documents in the composer. Files come only from the native picker (`client.pickDocuments`, Rust opens it)
 * or the composer's shared picker/paste/drop router
 * (the window reads the files' bytes and sends them with `addDropped`; Tauri's own drop handling is off); the
 * webview receives a project path only after `documents/save` has written original bytes. Every file
 * becomes a context chip. OCR starts on attach and every successful reader supplies model text. New chat holds attachments until its final folder exists.
 * Depends on: coreStore, composer (context chips, images, send guard), shellStore (selected session),
 * toastStore, composer image model, protocol types. Called by: createApp, the document views, the
 * `ui/documents` route.
 * Invariants: sending waits for the attachment read; failures carry their plain reason, never guessed text.
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
import { documentModelText as modelText, boundDocumentContext } from "./documentsText";
import type { ContextItem } from "../features/composer/model/message";

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
    {
      sessionId: string;
      path: string | null;
      contextId: string;
      pending: boolean;
      inFlight: boolean;
      document: DocumentView;
      body: string | null;
    }
  >();
  const replacing = new Set<string>();
  const reading = new Map<string, Promise<DocumentView | null>>();

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
    // Whose action it is (the facade); the command is the window's either way.
    _origin: "ui" | "debug-api",
    req: DocumentsRequest,
  ): Promise<DocumentsResult> => {
    const result = await core.run({ type: "documents", request: req });
    if (result.type !== "documents") throw new Error("unexpected answer to a documents request");
    return result.result;
  };

  /** Gives the agent the attachment path and the extracted document content when available. */
  const toContext = async (
    origin: "ui" | "debug-api",
    sessionId: string,
    doc: DocumentView,
    text: string | null,
    path: string | null,
  ): Promise<void> => {
    let body = text;
    if (body === null && doc.status === "ready") {
      const read = await request(origin, { op: "read", id: doc.id });
      body = read.kind === "document" ? read.text : null;
    }
    const old = saved.get(doc.id);
    if (old?.inFlight === true) {
      old.document = doc;
      old.body = body;
      old.pending = doc.status !== "ready";
      if (doc.status === "ready") dropChip(sessionId, doc.id);
      else putChip(sessionId, doc);
      return;
    }
    if (old !== undefined) {
      replacing.add(doc.id);
      composer.removeContext(old.sessionId, old.contextId);
    }
    const item = composer.addContext(sessionId, {
      label: doc.name,
      source: "document",
      text: body ?? doc.error ?? "Reading document text…",
      modelText: modelText(doc, path, body),
      ...(path === null ? {} : { documentPath: path }),
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
      document: doc,
      body,
    });
    replacing.delete(doc.id);
    if (doc.status === "ready") {
      dropChip(sessionId, doc.id);
      if (path !== null)
        await request(origin, { op: "release", id: doc.id }).catch(() => undefined);
    }
  };

  /** Places documents the user added into `sessionId`'s composer. */
  const place = async (origin: "ui" | "debug-api", sessionId: string, docs: DocumentView[]) => {
    const added: DocumentView[] = [];
    const refused: { name: string; reason: string }[] = [];
    for (const doc of docs) {
      try {
        if (sessionId === NEW_CHAT) {
          await toContext(origin, sessionId, doc, null, null);
          if (doc.status !== "ready") putChip(sessionId, doc);
          if (doc.status === "waiting") void actions(origin).extract(sessionId, doc.id);
          added.push(doc);
          continue;
        }
        const result = await request(origin, { op: "save", id: doc.id, sessionId });
        if (result.kind !== "saved") throw new Error("unexpected attachment save result");
        await toContext(origin, sessionId, doc, null, result.path);
        if (doc.status !== "ready") putChip(sessionId, doc);
        if (doc.status === "waiting") void actions(origin).extract(sessionId, doc.id);
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
    /** Places documents admitted by a native hand-over, using the same save and read-on-attach path. */
    attach(sessionId: string, docs: DocumentView[]) {
      return place(origin, sessionId, docs);
    },
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
    extract(_sessionId: string, id: string): Promise<DocumentView | null> {
      const existing = reading.get(id);
      if (existing !== undefined) return existing;
      setBusy(id, true);
      const pending = (async () => {
        try {
          if (deps.canUseKey?.() === false)
            throw new Error("Add or unlock your Mistral key in Settings, then try again.");
          const result = await request(origin, { op: "extract", id });
          if (result.kind !== "document")
            throw new Error("The document reader returned no result.");
          const item = saved.get(id);
          if (item === undefined) return null;
          await toContext(origin, item.sessionId, result.document, result.text, item.path);
          if (result.document.status !== "ready") putChip(item.sessionId, result.document);
          return result.document;
        } catch (thrown) {
          const item = saved.get(id);
          if (item === undefined) return null;
          const doc = {
            ...item.document,
            status: "failed",
            error:
              deps.canUseKey?.() === false
                ? "Add or unlock your Mistral key in Settings, then try again."
                : toCallError(thrown).error.message,
          };
          await toContext(origin, item.sessionId, doc, null, item.path);
          putChip(item.sessionId, doc);
          return doc;
        } finally {
          setBusy(id, false);
          reading.delete(id);
        }
      })();
      reading.set(id, pending);
      return pending;
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
      try {
        const result = await request(origin, { op: "takePicture", id });
        if (result.kind !== "picture") throw new Error("The document reader returned no picture.");
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
        composer.addImage(
          sessionId,
          origin === "debug-api" ? { ...image, by: "debug-api" } : image,
        );
        dropChip(sessionId, id);
        await request(origin, { op: "remove", id }).catch(() => undefined);
        return true;
      } catch (thrown) {
        composer.note(sessionId, `Not attached as a picture: ${toCallError(thrown).error.message}`);
        return false;
      }
    },
    /** Shows (or with `null` hides) the preview of a document context chip. */
    preview(target: DocumentsState["preview"]): void {
      store.set((s) => ({ ...s, preview: target }));
    },
  });

  /** Sending waits in prepareContext for reads already started on attach. */
  const blocking = (): string | null => null;
  composer.setSendGuard(blocking);

  composer.setPrepareContext(async (sessionId, context) => {
    const prepared: ContextItem[] = [];
    for (const chip of context) {
      const id = chip.uri?.startsWith("vw://doc/") ? chip.uri.slice("vw://doc/".length) : null;
      let item = id === null ? undefined : saved.get(id);
      if (id === null || item === undefined) {
        prepared.push(chip);
        continue;
      }
      await reading.get(id);
      const latest = saved.get(id);
      if (latest === undefined) throw new Error("The attachment was removed while reading.");
      item = latest;
      const path = item.path;
      const result =
        path === null
          ? await request("ui", { op: "save", id, sessionId })
          : { kind: "saved" as const, path };
      if (result.kind !== "saved")
        throw new Error("The attachment could not be saved. Try sending again.");
      const body = latest.body;
      prepared.push({
        ...chip,
        text: body ?? latest.document.error ?? chip.text,
        modelText: modelText(latest.document, result.path, body),
        documentPath: result.path,
      });
      if (item.pending) {
        dropChip(item.sessionId, id);
        putChip(sessionId, item.document);
      }
      item.sessionId = sessionId;
      item.path = result.path;
      if (!item.pending && path === null)
        await request("ui", { op: "release", id }).catch(() => undefined);
    }
    return boundDocumentContext(prepared);
  });

  const release = (
    id: string,
    item: { sessionId: string; pending: boolean; path: string | null },
    sent = false,
  ): void => {
    saved.delete(id);
    dropChip(item.sessionId, id);
    void request("ui", { op: sent ? "release" : "remove", id }).catch(() => undefined);
  };
  composer.setContentTaken((sessionId, context) => {
    for (const item of saved.values())
      if (item.sessionId === sessionId && context.some((chip) => chip.id === item.contextId))
        item.inFlight = true;
  });
  composer.setContentSettled((sessionId, context, sent) => {
    for (const [id, item] of saved) {
      if (!context.some((chip) => chip.id === item.contextId)) continue;
      if (item.sessionId !== sessionId) {
        if (item.pending) {
          dropChip(item.sessionId, id);
          putChip(sessionId, item.document);
        }
        item.sessionId = sessionId;
      }
      const restored = (composer.store.get().context[sessionId] ?? []).some(
        (chip) => chip.id === item.contextId,
      );
      item.inFlight = false;
      if (sent || !restored) release(id, item, sent);
      else void toContext("ui", sessionId, item.document, item.body, item.path);
    }
  });

  // Removing a context chip revokes its saved path; sending releases only the pending copy.
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
      void place("ui", selected ?? NEW_CHAT, event.added).then((placed) => {
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
