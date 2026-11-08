/** One feedback form's state and typed calls; captures and redacts before the person's deliberate Send. */
import type {
  FeedbackDraft,
  FeedbackRequest,
  FeedbackView,
  FeedbackScreenshot,
  FeedbackSystem,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";

/** An empty report contains no chat text or attachments. */
export function emptyFeedback(kind: FeedbackDraft["kind"] = "bug"): FeedbackDraft {
  return { kind, title: "", description: "", remarks: "" };
}
const trimmed = (draft: FeedbackDraft): FeedbackDraft => ({
  ...draft,
  title: draft.title.trim(),
  description: draft.description.trim(),
  remarks: draft.remarks.trim(),
});
const same = (a: FeedbackDraft, b: FeedbackDraft) =>
  a.kind === b.kind &&
  a.title === b.title &&
  a.description === b.description &&
  a.remarks === b.remarks;
const sameImage = (a: FeedbackScreenshot | null, b: FeedbackScreenshot | null) =>
  a === b ||
  (a !== null &&
    b !== null &&
    a.mimeType === b.mimeType &&
    a.dataBase64 === b.dataBase64 &&
    a.width === b.width &&
    a.height === b.height);
const sameSystem = (a: FeedbackSystem | null, b: FeedbackSystem | null) =>
  a === b ||
  (a !== null &&
    b !== null &&
    a.appVersion === b.appVersion &&
    a.vibeVersion === b.vibeVersion &&
    a.os === b.os &&
    a.osVersion === b.osVersion &&
    a.arch === b.arch);
const painted = () =>
  new Promise<void>((resolve, reject) => {
    let frame: number;
    const timeout = setTimeout(() => {
      cancelAnimationFrame(frame);
      reject(new Error("Screenshot unavailable. No screenshot attached."));
    }, 2000);
    frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => {
        clearTimeout(timeout);
        resolve();
      });
    });
  });
export function createFeedback(core: Pick<CoreController, "run" | "listen" | "trackAction">) {
  const store = createStore<{
    view: FeedbackView | null;
    draft: FeedbackDraft;
    busy: boolean;
    sending: boolean;
    capturing: boolean;
    previewing: boolean;
    error: string | null;
    notice: string | null;
    sent: boolean;
    reportId: string | null;
  }>({
    view: null,
    draft: emptyFeedback(),
    busy: false,
    sending: false,
    capturing: false,
    previewing: false,
    error: null,
    notice: null,
    sent: false,
    reportId: null,
  });
  function apply(view: FeedbackView, prepared = false) {
    store.set((s) => ({
      ...s,
      view,
      draft: prepared
        ? (view.review?.draft ?? view.draft ?? s.draft)
        : s.view?.draft != null
          ? s.draft
          : (view.draft ?? emptyFeedback()),
      previewing: view.screenshot !== null && s.previewing,
    }));
  }
  async function run(request: FeedbackRequest): Promise<boolean> {
    if (store.get().busy) return false;
    const previousView = store.get().view;
    store.set((s) => ({
      ...s,
      busy: true,
      sending: request.op === "send",
      capturing: request.op === "screenshot" && request.include,
      view:
        request.op === "screenshot" && s.view
          ? {
              ...s.view,
              addScreenshot: request.include,
              screenshot: null,
              screenshotError: null,
            }
          : s.view,
      error: null,
      notice: null,
      sent: false,
      reportId: null,
    }));
    try {
      // Let the hidden feedback overlay paint before the host takes native window pixels.
      if (request.op === "screenshot" && request.include) await painted();
      const result = await core.run({ type: "feedback", request });
      if (result.type !== "feedback") throw new Error("Feedback is unavailable.");
      apply(result.view, request.op === "review");
      // The core returns success only after the relay confirms this exact report id.
      if (request.op === "send")
        store.set((s) => ({ ...s, sent: true, reportId: request.reviewId }));
      return true;
    } catch (thrown) {
      store.set((s) => ({
        ...s,
        view: request.op === "screenshot" ? previousView : s.view,
        error: toCallError(thrown).error.message,
      }));
      return false;
    } finally {
      store.set((s) => ({ ...s, busy: false, sending: false, capturing: false }));
    }
  }
  async function send(): Promise<boolean> {
    const before = store.get();
    if (before.busy || !before.view?.available) return false;
    const entered = trimmed(before.draft);
    const expectedImage = before.view.screenshot;
    const expectedSystem = before.view.shareSystem
      ? (before.view.review?.system ?? before.view.system)
      : null;
    if (!(await run({ op: "review", draft: entered }))) return false;
    const report = store.get().view?.review;
    if (report == null) return false;
    if (!same(entered, report.draft)) {
      store.set((s) => ({
        ...s,
        previewing: false,
        notice: "Private text removed. Check the form.",
      }));
      return false;
    }
    if (
      !sameImage(expectedImage, report.screenshot ?? null) ||
      !sameSystem(expectedSystem, report.system)
    ) {
      store.set((s) => ({
        ...s,
        previewing: false,
        notice: "Attachments changed. Check the form.",
      }));
      return false;
    }
    return run({ op: "send", reviewId: report.id });
  }
  return {
    store,
    start() {
      const stop = core.listen((event) => {
        if (event.type === "feedback") apply(event.view);
        if (event.type === "lagged") void run({ op: "read" });
      });
      void run({ op: "read" });
      return stop;
    },
    open: (draft: FeedbackDraft = emptyFeedback()) =>
      store.get().view?.draft != null ? Promise.resolve(true) : run({ op: "open", draft }),
    edit(draft: FeedbackDraft) {
      store.set((s) => (s.busy ? s : { ...s, draft, error: null, notice: null }));
    },
    review: () => run({ op: "review", draft: store.get().draft }),
    share: (shareSystem: boolean) => run({ op: "share", shareSystem }),
    screenshot: (include: boolean) => core.trackAction(run({ op: "screenshot", include })),
    preview(previewing: boolean) {
      store.set((s) => ({ ...s, previewing }));
    },
    send: () => core.trackAction(send()),
    close: () => run({ op: "close" }),
    dismissSent: () => {
      store.set((s) => ({ ...s, sent: false, reportId: null }));
    },
  };
}
export type Feedback = ReturnType<typeof createFeedback>;
const controllers = new WeakMap<CoreController["store"], Feedback>();
/** One controller per core, including the composer's origin-bound facades. */
export function feedbackFor(core: CoreController): Feedback {
  let controller = controllers.get(core.store);
  if (controller === undefined) {
    controller = createFeedback(core);
    controllers.set(core.store, controller);
  }
  return controller;
}
