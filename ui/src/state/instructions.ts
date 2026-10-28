/**
 * The personal instruction editor's state and actions: the `AGENTS.md` in Vibe Work's own Vibe home.
 * Depends on: coreStore (`CoreController`), createStore, lineDiff, protocol/core types. Called by:
 * features/settings (InstructionsSettings, DocEditor), settingsRoutes.
 * Invariants:
 * - The core owns the file. This store sends only personal `instructions` commands, never a path.
 * - Nothing is written until the user has reviewed the diff and confirmed (`save` runs only from the
 *   review step; the file is replaced with exactly the reviewed draft, and the core refuses if the file
 *   changed after it was loaded).
 * - After a successful save the open sessions are told to take the new text (`apply`): Vibe builds a
 *   session's instructions once, so a used session reopens before its next message after current
 *   work has finished (proved against the pinned runtime's actual model requests).
 * - The CLI copy is "load into the editor, then review and save", so it uses the same safe path and
 *   never writes anywhere on its own.
 * - Failures are shown as plain text; none is swallowed.
 * - text a Debug API client put into an editor (`ui/settings` `type` / `loadCli`) is marked
 *   (`draftBy`), and its save and apply carry the Debug API origin even when the user clicks Save, so the core
 *   asks the native confirmation it asks the client itself. The mark is sticky through the user's own edits
 *   (one keystroke must not launder it, like the composer's) and ends when the draft is the document
 *   again: discard, a load that replaces the draft, a successful save, or typing it back.
 */
import type { InstructionsDoc } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { type Store, createStore } from "./createStore";
import { type DiffLine, lineDiff } from "./lineDiff";

/** Who wrote an editor's draft. */
type DraftBy = "user" | "debug-api";

/** One editor's state. */
export interface DocView {
  /** The document as the core last read or wrote it; `null` before the first read. */
  doc: InstructionsDoc | null;
  /** What the editor shows. */
  draft: string;
  /** `debug-api` once a Debug API client wrote into the draft (sticky, see the module docs). */
  draftBy: DraftBy;
  busy: "idle" | "loading" | "saving";
  /** A plain-words failure, or `null`. */
  error: string | null;
  /** The file changed on disk after it was loaded (the user must reload before saving). */
  stale: boolean;
  /** A plain-words result of the last action, or `null`. */
  notice: string | null;
  /** The review step is open. */
  reviewing: boolean;
}

/** Whole state. */
export interface InstructionsState {
  personal: DocView;
}

type InstructionsScope = "personal";

const EMPTY: DocView = {
  doc: null,
  draft: "",
  draftBy: "user",
  busy: "idle",
  error: null,
  stale: false,
  notice: null,
  reviewing: false,
};

/** True when the draft differs from the document. */
export function isDirty(view: DocView): boolean {
  return view.doc !== null && view.draft !== view.doc.text;
}

/** The diff the review step shows. */
export function reviewDiff(view: DocView): DiffLine[] {
  return lineDiff(view.doc?.text ?? "", view.draft);
}

/** What the controller offers. */
export interface Instructions {
  store: Store<InstructionsState>;
  /** Reads a document; keeps the user's unsaved draft unless `discard`. */
  load(scope: InstructionsScope, discard?: boolean): Promise<void>;
  /** Sets the draft; `by: "debug-api"` from the `ui/settings` route (marks it, module docs). */
  setDraft(scope: InstructionsScope, text: string, by?: DraftBy): void;
  /** Opens the review step (only with changes). */
  review(scope: InstructionsScope): void;
  /** Back to editing. */
  back(scope: InstructionsScope): void;
  /** Throws the draft away. */
  discard(scope: InstructionsScope): void;
  /** Writes the reviewed draft, then applies it to open sessions. */
  /** `by`: who pressed Save; a Debug API client saves only from the review step (the diff of what Save writes). */
  save(scope: InstructionsScope, by?: DraftBy): Promise<void>;
  /** Puts the user's command-line instructions into the personal editor (not saved yet); `by` as above. */
  loadCli(by?: DraftBy): Promise<void>;
}

function message(thrown: unknown): string {
  return toCallError(thrown).error.message;
}

/** Creates the controller for `core`. */
function createInstructions(core: CoreController): Instructions {
  const store = createStore<InstructionsState>({
    personal: EMPTY,
  });
  const patch = (scope: InstructionsScope, fields: Partial<DocView>) => {
    store.set((s) => ({ ...s, [scope]: { ...s[scope], ...fields } }));
  };

  const instructions: Instructions = {
    store,
    async load(scope, discard = false) {
      patch(scope, { busy: "loading", error: null });
      try {
        const result = await core.run({
          type: "instructions",
          request: { op: "read", scope },
        });
        if (result.type !== "instructions" || result.result.type !== "doc")
          throw new Error("unexpected answer");
        const doc = result.result.doc;
        const keep = !discard && isDirty(store.get()[scope]) && !store.get()[scope].stale;
        patch(scope, {
          doc,
          draft: keep ? store.get()[scope].draft : doc.text,
          draftBy: keep ? store.get()[scope].draftBy : "user",
          busy: "idle",
          stale: false,
          ...(discard ? { reviewing: false, notice: null } : {}),
        });
      } catch (thrown) {
        patch(scope, { busy: "idle", error: message(thrown) });
      }
    },
    setDraft(scope, text, by = "user") {
      const view = store.get()[scope];
      const draftBy: DraftBy =
        by === "debug-api" ? "debug-api" : text === view.doc?.text ? "user" : view.draftBy;
      patch(scope, { draft: text, draftBy, notice: null, error: null });
    },
    review(scope) {
      const view = store.get()[scope];
      if (view.doc !== null && view.doc.editable && isDirty(view))
        patch(scope, { reviewing: true });
    },
    back(scope) {
      patch(scope, { reviewing: false });
    },
    discard(scope) {
      const doc = store.get()[scope].doc;
      patch(scope, {
        draft: doc?.text ?? "",
        draftBy: "user",
        reviewing: false,
        error: null,
        notice: null,
        stale: false,
      });
    },
    async save(scope, by = "user") {
      const view = store.get()[scope];
      // The person saves changed text at once; a Debug API client, or text one wrote, only from the review step.
      if (view.doc === null || view.busy === "saving" || view.stale || !isDirty(view)) return;
      if (!view.reviewing && (by === "debug-api" || view.draftBy === "debug-api")) return;
      const text = view.draft;
      const expected = view.doc.exists ? view.doc.text : undefined;
      patch(scope, { busy: "saving", error: null, notice: null });
      try {
        const written = await core.run({
          type: "instructions",
          request: {
            op: "write",
            scope,
            text,
            ...(expected === undefined ? {} : { expected }),
          },
        });
        if (written.type !== "instructions" || written.result.type !== "doc")
          throw new Error("unexpected answer");
        const doc = written.result.doc;
        patch(scope, { doc, draft: doc.text, draftBy: "user", reviewing: false });
        const applied = await core
          .run({
            type: "instructions",
            request: { op: "apply", scope },
          })
          .catch(() => null);
        patch(scope, {
          busy: "idle",
          notice:
            applied === null
              ? "Saved. New chats receive the new text. Open chats could not be refreshed; close and reopen them to include it."
              : savedNotice(),
        });
      } catch (thrown) {
        const error = toCallError(thrown).error;
        patch(scope, {
          busy: "idle",
          error: error.message,
          stale: error.serverCode === "changedOnDisk",
          reviewing: false,
        });
      }
    },
    async loadCli(by = "user") {
      patch("personal", { busy: "loading", error: null, notice: null });
      try {
        const result = await core.run({ type: "instructions", request: { op: "readCli" } });
        if (result.type !== "instructions" || result.result.type !== "cli")
          throw new Error("unexpected answer");
        if (!result.result.available) {
          patch("personal", {
            busy: "idle",
            error: "There is no personal AGENTS.md in your Vibe command-line home.",
          });
          return;
        }
        const current = store.get().personal;
        // Added below what the editor holds (never replacing unsaved typing); a second press adds nothing.
        const cli = result.result.text;
        const draft = current.draft.includes(cli.trim())
          ? current.draft
          : current.draft.trim() === ""
            ? cli
            : `${current.draft.trimEnd()}\n\n${cli}`;
        patch("personal", {
          busy: "idle",
          draft,
          draftBy:
            by === "debug-api"
              ? "debug-api"
              : draft === current.doc?.text
                ? "user"
                : current.draftBy,
          notice: null,
        });
      } catch (thrown) {
        patch("personal", { busy: "idle", error: message(thrown) });
      }
    },
  };
  return instructions;
}

/** The status shown after a successful save. */
export function savedNotice(): string {
  return "Saved.";
}

/** One controller per core (the settings screens and the UI-control route share it). */
const controllers = new WeakMap<CoreController, Instructions>();

/** The shared controller for `core`. */
export function instructionsFor(core: CoreController): Instructions {
  let found = controllers.get(core);
  if (found === undefined) {
    found = createInstructions(core);
    controllers.set(core, found);
  }
  return found;
}
