/**
 * The Capture menu's controller: what the Browser tab can capture from the page shown (the same
 * `browser` `capture` command the agent's `browser_capture` runs), the capture running per session, and the last
 * result (the folder inside the session folder, written filenames and count, what was skipped). The files land in
 * `<session folder>/capture/<page>-<time>/`; the core decides everything about paths and contents.
 * Depends on: state/coreStore (`run`), state/createStore, protocol types. Called by: CaptureMenu, browserRoutes.ts
 * (`capture` action), register.tsx, UI tests.
 * Invariant: one capture at a time per session (a second request while one runs is refused here, not queued).
 */
import { toCallError } from "../../protocol/client/types";
import type { BrowserResult, JsonValue } from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";
import { createStore } from "../../state/createStore";

/** One entry of the menu. */
export interface CaptureKind {
  id: string;
  label: string;
  hint: string;
  args: { what: string; formats?: string[] };
  /** Starts a group (a separator above it). */
  group?: boolean;
}

/** The menu, in order. */
export const CAPTURE_KINDS: readonly CaptureKind[] = [
  {
    id: "design",
    label: "Design system",
    hint: "DESIGN.md and tokens.json",
    args: { what: "design" },
  },
  {
    id: "structure",
    label: "Structure",
    hint: "Elements with layout boxes",
    args: { what: "structure" },
  },
  { id: "media", label: "Media", hint: "Logo, icons, images, fonts", args: { what: "media" } },
  {
    id: "page",
    label: "Whole page",
    hint: "Markdown, text, HTML, full-page PNG",
    args: { what: "page" },
    group: true,
  },
  {
    id: "markdown",
    label: "Markdown",
    hint: "page.md",
    args: { what: "page", formats: ["markdown"] },
  },
  { id: "text", label: "Plain text", hint: "page.txt", args: { what: "page", formats: ["text"] } },
  {
    id: "html",
    label: "HTML without scripts",
    hint: "page.html",
    args: { what: "page", formats: ["html"] },
  },
  {
    id: "png",
    label: "Full-page picture",
    hint: "page.png",
    args: { what: "page", formats: ["png"] },
  },
  {
    id: "bundle",
    label: "Everything",
    hint: "All of the above in one folder",
    args: { what: "bundle" },
    group: true,
  },
  {
    id: "before",
    label: "Before shot",
    hint: "To compare with after a change",
    args: { what: "before" },
    group: true,
  },
  {
    id: "after",
    label: "After shot and compare",
    hint: "after.png and diff.png",
    args: { what: "after" },
  },
];

/** A finished capture as the tab shows it. */
export interface CaptureView {
  kind: string;
  folder: string;
  path: string;
  outcome: string;
  files: number;
  fileNames: string[];
  skipped: number;
}

/** The Capture menu's state. */
export interface CaptureUi {
  /** The label of the capture running in this session. */
  busy: Readonly<Record<string, string | undefined>>;
  last: Readonly<Record<string, CaptureView | undefined>>;
  error: Readonly<Record<string, string | null>>;
}

function field(value: JsonValue | undefined, key: string): JsonValue | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value[key]
    : undefined;
}

/** Reads the parts the tab shows from a capture answer (unknown shapes give empty values). */
export function captureView(kind: string, answer: JsonValue): CaptureView {
  const text = (key: string) => {
    const v = field(answer, key);
    return typeof v === "string" ? v : "";
  };
  const list = (key: string) => {
    const v = field(answer, key);
    return Array.isArray(v) ? v.length : 0;
  };
  const media = field(answer, "mediaFiles");
  const files = field(answer, "files");
  const namedMedia = Array.isArray(files)
    ? files.filter((file) => {
        const name = field(file, "name");
        return typeof name === "string" && name.startsWith("media/");
      }).length
    : 0;
  return {
    kind,
    folder: text("folder"),
    path: text("path"),
    outcome: text("Outcome"),
    files: list("files") + Math.max(0, (typeof media === "number" ? media : 0) - namedMedia),
    fileNames: Array.isArray(files)
      ? files.flatMap((file) => {
          const name = field(file, "name");
          return typeof name === "string" ? [name] : [];
        })
      : [],
    skipped: list("skipped"),
  };
}

/** Creates the controller for `core`. */
export function createCapture(core: CoreController) {
  const store = createStore<CaptureUi>({ busy: {}, last: {}, error: {} });
  const set = (
    sessionId: string,
    patch: Partial<{ busy: string | undefined; last: CaptureView; error: string | null }>,
  ) => {
    store.set((s) => ({
      busy: "busy" in patch ? { ...s.busy, [sessionId]: patch.busy } : s.busy,
      last: patch.last !== undefined ? { ...s.last, [sessionId]: patch.last } : s.last,
      error: "error" in patch ? { ...s.error, [sessionId]: patch.error ?? null } : s.error,
    }));
  };
  return {
    store,
    /** Runs the capture `kindId` of the menu in `sessionId`; answers the result, or `null` (the error is kept). */
    async capture(
      sessionId: string,
      kindId: string,
      extra: Record<string, string> = {},
      tabId?: string,
    ): Promise<CaptureView | null> {
      const kind = CAPTURE_KINDS.find((k) => k.id === kindId);
      if (kind === undefined) {
        set(sessionId, { error: `unknown capture ${kindId}` });
        return null;
      }
      if (store.get().busy[sessionId] !== undefined) {
        set(sessionId, { error: "A capture is already running in this session." });
        return null;
      }
      set(sessionId, { busy: kind.label, error: null });
      try {
        const result: BrowserResult = await core
          .run({
            type: "browser",
            request: {
              op: "capture",
              sessionId,
              ...(tabId ? { tabId } : {}),
              args: { ...kind.args, ...extra },
            },
          })
          .then((r) => {
            if (r.type !== "browser") throw new Error(`unexpected ${r.type} result`);
            return r.result;
          });
        if (result.type !== "capture") throw new Error(`unexpected ${result.type} answer`);
        const view = captureView(kind.id, result.capture);
        set(sessionId, { last: view, busy: undefined });
        return view;
      } catch (thrown) {
        set(sessionId, { busy: undefined, error: toCallError(thrown).error.message });
        return null;
      }
    },
    /** Hides the last result line. */
    dismiss(sessionId: string): void {
      store.set((s) => ({
        ...s,
        last: Object.fromEntries(Object.entries(s.last).filter(([id]) => id !== sessionId)),
        error: { ...s.error, [sessionId]: null },
      }));
    },
  };
}

/** The controller type. */
export type Capture = ReturnType<typeof createCapture>;
