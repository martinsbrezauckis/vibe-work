/**
 * The Libraries page's state and actions: the user's
 * Mistral libraries with who may use them here (off, all chats, one project), the selected library's documents, the
 * upload (native picker in the app, bytes from a drop or the Debug API), the Ask box with numbered sources, and the
 * text view a source opens.
 * Depends on: coreStore (`run`), protocol/core (work and projects types), toastStore, shellStore (a session selection
 * closes the page). Called by: createApp, the library views, the `ui/libraries` route.
 * Invariants: every command a Debug API route triggers carries the Debug API origin (`actions("debug-api")`); file
 * text, names, summaries and answers are shown as text only (they come from the user's files); while a document of
 * the selected library is still processing on Mistral's side the list is read again every 3 s, at most 40 times.
 */
import type {
  LibrariesRequest,
  LibrariesResult,
  LibraryDocumentView,
  LibraryFile,
  LibraryScope,
  LibrarySourceView,
  LibraryView,
  ProjectView,
  WorkCosts,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreClient } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";

/** An answer to a question across libraries. */
export interface LibraryAnswer {
  question: string;
  answer: string;
  sources: readonly LibrarySourceView[];
  queries: readonly string[];
  tokens: number;
}

/** A document's text, opened from the list or from a source. */
export interface LibraryViewer {
  name: string;
  text: string;
  truncated: boolean;
  /** The source number it was opened from, if any. */
  source: number | null;
}

/** Everything the page shows. */
export interface LibrariesState {
  open: boolean;
  /** `null` until the first list arrived. */
  libraries: readonly LibraryView[] | null;
  costs: WorkCosts | null;
  /** Projects for the scope picker (id, name). */
  projects: readonly Pick<ProjectView, "id" | "name">[];
  loading: boolean;
  /** Why the last read failed. */
  error: string | null;
  selected: string | null;
  /** Documents per library id. */
  documents: Readonly<Record<string, readonly LibraryDocumentView[]>>;
  /** What is running ("upload", "ask", ...), or null. */
  busy: string | null;
  /** A short outcome line. */
  note: string | null;
  question: string;
  answer: LibraryAnswer | null;
  viewer: LibraryViewer | null;
}

export const INITIAL_LIBRARIES: LibrariesState = {
  open: false,
  libraries: null,
  costs: null,
  projects: [],
  loading: false,
  error: null,
  selected: null,
  documents: {},
  busy: null,
  note: null,
  question: "",
  answer: null,
  viewer: null,
};

/** True while Mistral still processes the document (not yet searchable). */
export function processing(doc: LibraryDocumentView): boolean {
  return (
    doc.status === "todo" || doc.status === "in_progress" || doc.status === "waiting_for_capacity"
  );
}

/** The status in words. */
export function statusText(doc: LibraryDocumentView): string {
  switch (doc.status) {
    case "done":
      return "Ready";
    case "todo":
    case "in_progress":
    case "waiting_for_capacity":
      return "Processing…";
    case "error":
      return "Mistral could not read it";
    case "missing_content":
      return "No text found";
    default:
      return doc.status;
  }
}

/** The scope in words. */
export function scopeText(scope: LibraryScope, projects: LibrariesState["projects"]): string {
  if (scope.kind === "off") return "Not used in chats";
  if (scope.kind === "global") return "All chats";
  const name = projects.find((p) => p.id === scope.projectId)?.name;
  return name === undefined ? "A project (deleted?)" : `Chats in ${name}`;
}

/** Ms between status reads while a document processes, and how many at most. */
const POLL_MS = 3000;
const POLL_MAX = 40;

/** Creates the libraries controller. */
export function createLibraries(deps: {
  core: CoreController;
  client: CoreClient;
  shell: Shell;
  keyReady?: () => boolean;
}) {
  const { core, client, shell } = deps;
  const store = createStore<LibrariesState>(INITIAL_LIBRARIES);
  const patch = (p: Partial<LibrariesState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  let polls = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const request = async (
    origin: "ui" | "debug-api",
    req: LibrariesRequest,
  ): Promise<LibrariesResult> => {
    if (origin === "ui" && deps.keyReady?.() === false) throw new Error("Mistral key required");
    const result = await core.run(
      { type: "libraries", request: req },
      origin === "debug-api" ? { origin } : undefined,
    );
    if (result.type !== "libraries") throw new Error("unexpected answer to a libraries request");
    return result.result;
  };

  const fail = (thrown: unknown): void => {
    if (deps.keyReady?.() === false) {
      patch({ busy: null, error: null, note: null });
      return;
    }
    const message = toCallError(thrown).error.message;
    patch({ note: message.charAt(0).toUpperCase() + message.slice(1), busy: null });
  };

  const actions = (origin: "ui" | "debug-api") => {
    const documents = async (id: string): Promise<void> => {
      const result = await request(origin, { op: "documents", libraryId: id });
      if (result.kind !== "documents") return;
      store.set((s) => ({ ...s, documents: { ...s.documents, [id]: result.documents } }));
      const waiting = result.documents.some(processing);
      if (timer !== null) clearTimeout(timer);
      timer = null;
      if (waiting && polls < POLL_MAX && store.get().open && store.get().selected === id) {
        polls += 1;
        timer = setTimeout(() => {
          void documents(id).catch(() => undefined);
        }, POLL_MS);
      }
    };
    const load = async (): Promise<void> => {
      if (origin === "ui" && deps.keyReady?.() === false) {
        patch({ loading: false, error: null, note: null });
        return;
      }
      patch({ loading: true });
      try {
        const result = await request(origin, { op: "list" });
        if (result.kind === "list")
          patch({ libraries: result.libraries, costs: result.costs, error: null });
        const projects = await core.run(
          { type: "projects", request: { op: "list" } },
          origin === "debug-api" ? { origin } : undefined,
        );
        if (projects.type === "projects" && projects.result.type === "list")
          patch({ projects: projects.result.projects.map((p) => ({ id: p.id, name: p.name })) });
      } catch (thrown) {
        if (origin === "ui" && deps.keyReady?.() === false) {
          patch({ error: null });
          return;
        }
        // The core's sentences start lower-case (they also go to logs); the page shows them as sentences.
        const message = toCallError(thrown).error.message;
        patch({ error: message.charAt(0).toUpperCase() + message.slice(1) });
      } finally {
        patch({ loading: false });
      }
    };
    const uploaded = async (id: string, result: LibrariesResult): Promise<void> => {
      if (result.kind !== "uploaded") return;
      const n = result.added.length;
      const refused = result.refused.map((r) => `${r.name}: ${r.reason}`).join("; ");
      const added =
        n === 0 ? "" : `Added ${String(n)} file${n === 1 ? "" : "s"}; Mistral is processing them.`;
      patch({
        note:
          [added, refused === "" ? "" : `Not added: ${refused}`]
            .filter((x) => x !== "")
            .join(" ") || null,
        busy: null,
      });
      polls = 0;
      await documents(id);
      await load();
    };
    return {
      /** Opens the page and reads the libraries. */
      async open(): Promise<void> {
        patch({ open: true, note: null });
        if (origin === "ui" && deps.keyReady?.() === false) return;
        await load();
        const first = store.get().selected ?? store.get().libraries?.[0]?.id ?? null;
        if (first !== null) await this.select(first);
      },
      close(): void {
        if (timer !== null) clearTimeout(timer);
        patch({ open: false, viewer: null });
      },
      load,
      /** Shows a library and reads its documents. */
      async select(id: string | null): Promise<void> {
        patch({ selected: id, viewer: null });
        polls = 0;
        if (id === null || (origin === "ui" && deps.keyReady?.() === false)) return;
        try {
          await documents(id);
        } catch (thrown) {
          fail(thrown);
        }
      },
      /** Makes a library (it starts as used in all chats unless another scope is given). */
      async create(
        name: string,
        scope: LibraryScope,
        description: string | null,
      ): Promise<boolean> {
        patch({ busy: "create", note: null });
        try {
          const result = await request(origin, { op: "create", name, scope, description });
          await load();
          if (result.kind === "library") await this.select(result.library.id);
          patch({ busy: null, note: `Made the library ${name.trim()}.` });
          return true;
        } catch (thrown) {
          fail(thrown);
          return false;
        }
      },
      async setScope(id: string, scope: LibraryScope): Promise<void> {
        try {
          await request(origin, { op: "setScope", libraryId: id, scope });
          await load();
          patch({ note: null });
        } catch (thrown) {
          fail(thrown);
        }
      },
      async deleteLibrary(id: string): Promise<void> {
        patch({ busy: "delete", note: null });
        try {
          await request(origin, { op: "delete", libraryId: id });
          if (store.get().selected === id) patch({ selected: null });
          await load();
          patch({ busy: null, note: "The library and its documents are deleted on Mistral." });
        } catch (thrown) {
          fail(thrown);
        }
      },
      /** The native picker (app only). */
      async pick(id: string): Promise<void> {
        if (deps.keyReady?.() === false) return;
        patch({ busy: "upload", note: null });
        try {
          const result = await client.pickLibraryFiles(id);
          if (result.type === "libraries") await uploaded(id, result.result);
          else patch({ busy: null });
        } catch (thrown) {
          fail(thrown);
        }
      },
      /** Files whose bytes the client holds (a drop, the Debug API). */
      async uploadFiles(id: string, files: LibraryFile[]): Promise<void> {
        patch({ busy: "upload", note: null });
        try {
          await uploaded(id, await request(origin, { op: "upload", libraryId: id, files }));
        } catch (thrown) {
          fail(thrown);
        }
      },
      async deleteDocument(libraryId: string, documentId: string): Promise<void> {
        try {
          await request(origin, { op: "deleteDocument", libraryId, documentId });
          await documents(libraryId);
          await load();
        } catch (thrown) {
          fail(thrown);
        }
      },
      /** Opens a document's text (from the list, or from source `n` of the answer). */
      async openText(
        libraryId: string,
        documentId: string,
        source: number | null = null,
      ): Promise<void> {
        patch({ busy: "text" });
        try {
          const result = await request(origin, { op: "text", libraryId, documentId });
          if (result.kind === "text")
            patch({
              viewer: { name: result.name, text: result.text, truncated: result.truncated, source },
            });
          patch({ busy: null });
        } catch (thrown) {
          fail(thrown);
        }
      },
      closeViewer(): void {
        patch({ viewer: null });
      },
      setQuestion(question: string): void {
        patch({ question });
      },
      /** Asks across every library that is not off (or the given ones). */
      async ask(question: string, libraryIds: string[] = []): Promise<void> {
        patch({ busy: "ask", note: null, question });
        try {
          const result = await request(origin, { op: "ask", question, libraryIds });
          if (result.kind === "answer")
            patch({
              answer: {
                question,
                answer: result.answer,
                sources: result.sources,
                queries: result.queries,
                tokens: result.tokens,
              },
            });
          patch({ busy: null });
        } catch (thrown) {
          fail(thrown);
        }
      },
    };
  };

  // Picking a session in the list shows that session instead of the page.
  let selected = shell.store.get().selected;
  shell.store.subscribe(() => {
    const now = shell.store.get().selected;
    if (now !== selected && store.get().open) patch({ open: false, viewer: null });
    selected = now;
  });

  return { store, actions: actions("ui"), debugApi: actions("debug-api") } as const;
}

/** The libraries controller. */
export type Libraries = ReturnType<typeof createLibraries>;
