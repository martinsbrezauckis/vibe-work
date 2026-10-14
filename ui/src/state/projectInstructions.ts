/**
 * Project instruction files and reviewed drafts. Depends on the typed core client and shared
 * DocView; called by ProjectSettings. Names stay in AGENTS.md or docs/instructions, and edits,
 * creation and link changes write only after review and Save. Core checks folder identity,
 * stale files and user-only admission; drafts survive closing and reopening the settings pane.
 */
import type { ProjectInstructionFile, ProjectInstructionsRequest } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import { type DocView, isDirty } from "./instructions";

const emptyView = (): DocView => ({
  doc: null,
  draft: "",
  draftBy: "user",
  busy: "idle",
  error: null,
  stale: false,
  notice: null,
  reviewing: false,
});

/** Portable Markdown basename; paths are never accepted by the name field. */
export function instructionNameError(name: string): string | null {
  if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]*\.md$/i.test(name) || name.length > 96)
    return "Use a Markdown file name, such as code-style.md (letters, numbers, - or _).";
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])\.md$/i.test(name))
    return "This file name is reserved by the operating system.";
  if (name.toLowerCase() === "agents.md")
    return "Use the AGENTS.md entry above to create or edit Vibe's main instructions.";
  return null;
}

/** Creates a controller bound to one saved project and main-folder identity. */
export function createProjectInstructions(core: CoreController, projectId: string, folder: string) {
  const store = createStore({
    files: [] as ProjectInstructionFile[],
    loading: false,
    error: null as string | null,
    name: null as string | null,
    view: emptyView(),
  });
  const patchView = (fields: Partial<DocView>) => {
    store.set((s) => ({ ...s, view: { ...s.view, ...fields } }));
  };
  const run = async (request: ProjectInstructionsRequest) => {
    const result = await core.run({ type: "projectInstructions", request });
    if (result.type !== "projectInstructions") throw new Error("Unexpected instruction result.");
    return result.result;
  };
  const error = (thrown: unknown) => {
    const problem = toCallError(thrown).error;
    patchView({
      busy: "idle",
      error: problem.message,
      stale: problem.serverCode === "changedOnDisk",
      reviewing: false,
    });
  };
  const canOpen = () => {
    const view = store.get().view;
    if (view.busy !== "idle" || store.get().loading) return false;
    if (isDirty(view)) {
      patchView({ error: "Save or discard your current changes before opening another file." });
      return false;
    }
    return true;
  };
  const controller = {
    store,
    async refresh() {
      store.set((s) => ({ ...s, loading: true, error: null }));
      try {
        const result = await run({ op: "list", projectId });
        if (result.type !== "list" || result.folder !== folder)
          throw new Error("The project's main folder changed. Reopen Project settings.");
        store.set((s) => ({ ...s, files: result.files, loading: false }));
      } catch (thrown) {
        store.set((s) => ({ ...s, loading: false, error: toCallError(thrown).error.message }));
      }
    },
    async open(name: string, discard = false) {
      if (!discard && !canOpen()) return;
      store.set((s) => ({ ...s, name, view: { ...emptyView(), busy: "loading" } }));
      try {
        const result = await run({ op: "read", projectId, name });
        if (result.type !== "doc" || result.doc.folder !== folder)
          throw new Error("The project's main folder changed. Reopen Project settings.");
        patchView({ doc: result.doc, draft: result.doc.text, busy: "idle" });
      } catch (thrown) {
        error(thrown);
      }
    },
    async newFile(basename: string) {
      const problem = instructionNameError(basename);
      const name = `docs/instructions/${basename}`;
      if (problem !== null) {
        patchView({ error: problem });
        return false;
      }
      if (store.get().files.some((f) => f.name.toLowerCase() === name.toLowerCase())) {
        patchView({ error: "An instruction file with this name already exists." });
        return false;
      }
      if (!canOpen()) return false;
      await controller.open(name);
      return store.get().view.doc !== null;
    },
    setDraft(draft: string) {
      patchView({ draft, notice: null, error: null });
    },
    review() {
      const view = store.get().view;
      if (view.doc?.editable && isDirty(view) && view.busy === "idle" && !view.stale)
        patchView({ reviewing: true });
    },
    back() {
      patchView({ reviewing: false });
    },
    discard() {
      patchView({
        draft: store.get().view.doc?.text ?? "",
        reviewing: false,
        error: null,
        notice: null,
      });
    },
    async reload() {
      const name = store.get().name;
      if (name !== null) await controller.open(name, true);
    },
    async save() {
      const { name, view } = store.get();
      if (
        name === null ||
        view.doc === null ||
        !view.reviewing ||
        view.busy !== "idle" ||
        view.stale
      )
        return;
      patchView({ busy: "saving", error: null, notice: null });
      try {
        const common = { projectId, name, expectedFolder: folder, text: view.draft };
        const result = await run(
          view.doc.exists
            ? { op: "write", ...common, expected: view.doc.text }
            : { op: "create", ...common },
        );
        if (result.type !== "doc") throw new Error("Unexpected save result.");
        patchView({
          doc: result.doc,
          draft: result.doc.text,
          busy: "idle",
          reviewing: false,
          notice:
            name === "AGENTS.md" || store.get().files.some((f) => f.name === name && f.linked)
              ? "Saved. Open sessions use updated instructions on their next message."
              : "Saved. Link this file from AGENTS.md so the agent knows to read it.",
        });
        await controller.refresh();
      } catch (thrown) {
        error(thrown);
      }
    },
    async link(name: string, linked: boolean) {
      if (!canOpen()) return;
      store.set((s) => ({ ...s, name: "AGENTS.md", view: { ...emptyView(), busy: "loading" } }));
      try {
        const result = await run({ op: "linkDraft", projectId, name, linked });
        if (result.type !== "linkDraft" || result.doc.folder !== folder)
          throw new Error("The project's main folder changed. Reopen Project settings.");
        patchView({
          doc: result.doc,
          draft: result.draft,
          busy: "idle",
          notice: "Review this AGENTS.md change, then Save to apply it.",
        });
      } catch (thrown) {
        error(thrown);
      }
    },
    async remove(name: string) {
      if (!canOpen()) return;
      patchView({ busy: "saving", error: null });
      try {
        const loaded = await run({ op: "read", projectId, name });
        if (
          loaded.type !== "doc" ||
          loaded.doc.folder !== folder ||
          !loaded.doc.exists ||
          !loaded.doc.editable
        )
          throw new Error("This file cannot be deleted. Read the file list again.");
        const result = await run({
          op: "delete",
          projectId,
          name,
          expectedFolder: folder,
          expected: loaded.doc.text,
        });
        if (result.type === "deleted") {
          store.set((s) => ({ ...s, name: null, view: emptyView() }));
          await controller.refresh();
        } else {
          patchView({ busy: "idle" });
        }
      } catch (thrown) {
        error(thrown);
      }
    },
  };
  return controller;
}

/** Controller type consumed by the settings pane. */
export type ProjectInstructions = ReturnType<typeof createProjectInstructions>;
const controllers = new WeakMap<CoreController, Map<string, ProjectInstructions>>();

/** Reuses unsaved drafts for the same project and main folder. */
export function projectInstructionsFor(core: CoreController, projectId: string, folder: string) {
  let map = controllers.get(core);
  if (map === undefined) {
    map = new Map();
    controllers.set(core, map);
  }
  const key = JSON.stringify([projectId, folder]);
  let controller = map.get(key);
  if (controller === undefined) {
    controller = createProjectInstructions(core, projectId, folder);
    map.set(key, controller);
  }
  return controller;
}
