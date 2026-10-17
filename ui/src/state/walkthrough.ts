/** Debug walkthrough state. Depends on createStore; the caption and Debug API share this per-window state. */
import { createStore } from "./createStore";

/** Caption state; normal windows start off. */
export interface WalkthroughState {
  phase: "off" | "running" | "paused" | "finished";
  step: number;
  total: number;
  label: string;
}

/** Creates a walkthrough whose final caption expires after three seconds. */
export function createWalkthrough() {
  const store = createStore<WalkthroughState>({ phase: "off", step: 0, total: 0, label: "" });
  let timer: ReturnType<typeof setTimeout> | undefined;
  return {
    store,
    start(total: number): void {
      clearTimeout(timer);
      store.set({ phase: "running", step: 0, total, label: "" });
    },
    step(step: number, label: string): void {
      if (store.get().phase === "running") store.set((s) => ({ ...s, step, label }));
    },
    pause(): void {
      if (store.get().phase === "running") store.set((s) => ({ ...s, phase: "paused" }));
    },
    finish(): void {
      clearTimeout(timer);
      store.set((s) => ({ ...s, phase: "finished" }));
      timer = setTimeout(() => {
        this.stop();
      }, 3000);
    },
    stop(): void {
      clearTimeout(timer);
      store.set((s) => ({ ...s, phase: "off" }));
    },
  };
}

/** The one-line caption; no result text appears in the window. */
export function walkthroughText(s: WalkthroughState): string {
  if (s.phase === "off") return "";
  if (s.phase === "paused") return `Walkthrough paused at step ${String(s.step)}`;
  if (s.phase === "finished") return `Walkthrough finished · ${String(s.total)} steps`;
  return `Step ${String(s.step)} of ${String(s.total)}${s.label === "" ? "" : ` · ${s.label}`}`;
}
