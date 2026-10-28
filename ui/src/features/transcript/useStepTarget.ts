/** Bell navigation resolves steps inside virtual groups, then uses the search highlight and moves focus. */
import { useEffect, useRef, type RefObject } from "react";
import type { VListHandle } from "virtua";
import { useStore } from "../../state/createStore";
import { useApp } from "../shell/AppContext";
import { rowEntries, type Row } from "./model/build";
import { entryId } from "./model/entries";

/** Resolve a history id or tool-call id without using a search query or command arguments. */
export function useStepTarget(
  sessionId: string,
  rows: readonly Row[],
  list: RefObject<VListHandle | null>,
  pauseFollow: () => void,
) {
  const app = useApp();
  const target = useStore(app.find.store, (state) => state.step);
  const read = useRef<object | null>(null);
  const matches = (entry: Record<string, unknown>) =>
    target !== null &&
    (entry["id"] === target.stepId ||
      entry["toolCallId"] === target.stepId ||
      (typeof entry["detail"] === "object" &&
        entry["detail"] !== null &&
        "toolCallId" in entry["detail"] &&
        entry["detail"].toolCallId === target.stepId));
  const index =
    target?.sessionId === sessionId ? rows.findIndex((row) => rowEntries(row).some(matches)) : -1;
  const row = rows[index];
  const found = row === undefined ? undefined : rowEntries(row).find(matches);
  const id = found === undefined ? null : entryId(found);
  const group = row?.kind === "steps" ? row.key : null;
  useEffect(() => {
    if (target === null || target.sessionId !== sessionId) return;
    if (index < 0) {
      if (read.current !== target) {
        read.current = target;
        void app.core
          ?.run({
            type: "call",
            method: "session/read",
            params: { sessionId, history: { limit: 500 }, turns: { limit: 500 } },
          })
          .catch(() => undefined);
      }
      return;
    }
    pauseFollow();
    if (group !== null) app.transcript.toggle(group, true);
    if (id !== null) app.transcript.toggle(`step:${id}`, true);
    list.current?.scrollToIndex(index, { align: "center" });
    let element: HTMLElement | undefined;
    const timer = window.setTimeout(() => {
      element = [...document.querySelectorAll<HTMLElement>(".tx-list [data-entry-id]")].find(
        (node) => node.dataset["entryId"] === id,
      );
      if (element === undefined) return;
      element.classList.add("find-current");
      element.tabIndex = -1;
      element.focus({ preventScroll: true });
    }, 60);
    return () => {
      window.clearTimeout(timer);
      element?.classList.remove("find-current");
    };
  }, [app, sessionId, index, id, group, list, target, pauseFollow]);
}
