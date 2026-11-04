/** Feedback panel state and typed core calls; components edit prose and a single sharing choice. */
import type { FeedbackDraft, FeedbackRequest, FeedbackView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";

/** An empty report contains no chat text or attachments. */
export function emptyFeedback(kind: FeedbackDraft["kind"] = "bug"): FeedbackDraft {
  return { kind, title: "", description: "", remarks: "" };
}
export function createFeedback(core: Pick<CoreController, "run" | "listen">) {
  const store = createStore<{
    view: FeedbackView | null;
    draft: FeedbackDraft;
    step: "write" | "review";
    busy: boolean;
    sending: boolean;
    error: string | null;
    sent: boolean;
  }>({
    view: null,
    draft: emptyFeedback(),
    step: "write",
    busy: false,
    sending: false,
    error: null,
    sent: false,
  });
  function apply(view: FeedbackView) {
    store.set((s) => {
      const changedReview = view.review?.id !== s.view?.review?.id;
      return {
        ...s,
        view,
        draft:
          changedReview && view.review !== null
            ? view.review.draft
            : s.view?.draft != null
              ? s.draft
              : (view.draft ?? s.draft),
        step: changedReview ? (view.review === null ? "write" : "review") : s.step,
      };
    });
  }
  async function run(request: FeedbackRequest): Promise<boolean> {
    if (store.get().busy) return false;
    store.set((s) => ({
      ...s,
      busy: true,
      sending: request.op === "send",
      error: null,
      sent: false,
    }));
    try {
      const result = await core.run({ type: "feedback", request });
      if (result.type !== "feedback") throw new Error("Feedback is unavailable.");
      apply(result.view);
      if (request.op === "send") store.set((s) => ({ ...s, sent: true }));
      return true;
    } catch (thrown) {
      store.set((s) => ({ ...s, error: toCallError(thrown).error.message }));
      return false;
    } finally {
      store.set((s) => ({ ...s, busy: false, sending: false }));
    }
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
    open: (kind: FeedbackDraft["kind"] = "bug") => run({ op: "open", draft: emptyFeedback(kind) }),
    edit(draft: FeedbackDraft) {
      store.set((s) => (s.busy ? s : { ...s, draft, step: "write", error: null }));
    },
    back() {
      store.set((s) => ({ ...s, step: "write" }));
    },
    review: () => run({ op: "review", draft: store.get().draft }),
    share: (shareSystem: boolean) => run({ op: "share", shareSystem }),
    send: () => {
      const { view, step } = store.get();
      return view?.review == null || step !== "review"
        ? Promise.resolve(false)
        : run({ op: "send", reviewId: view.review.id });
    },
    close: () => run({ op: "close" }),
    dismissSent: () => {
      store.set((s) => ({ ...s, sent: false }));
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
