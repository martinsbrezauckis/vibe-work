/**
 * A rendered shell with one session open in the transcript: pushes a snapshot with
 * the given history and turns through the fake client, selects the session, and offers helpers to
 * patch entries and open callbacks as the core would. Synthetic data only.
 */
import { act } from "@testing-library/react";

import type { CoreEvent, JsonValue } from "../../../../ui/src/protocol/core";
import { snapshot } from "./fakeClient";
import { type Rig, renderShell } from "./renderShell";

export interface TranscriptRig extends Rig {
  /** Applies a JSON patch to one entry (`entryPatched`). */
  patch(entryId: string, ops: JsonValue[]): void;
  /** Adds or replaces an entry (`entryUpserted`). */
  upsert(entry: JsonValue): void;
  /** Sets the core's open callbacks (`callbacksChanged`). */
  openCallbacks(ids: string[]): void;
  /** Lets effects and promises settle. */
  settle(ms?: number): Promise<void>;
}

/** Renders the shell with session `s1` showing `history` / `turns`. */
export async function transcriptRig(
  history: JsonValue[],
  turns: JsonValue[] = [],
  options: {
    status?: unknown;
    openCallbackIds?: string[];
    persist?: boolean;
    shell?: Parameters<typeof renderShell>[0];
    /** Extra fields of the session object (for example `title`). */
    session?: Record<string, unknown>;
  } = {},
): Promise<TranscriptRig> {
  const rig = await renderShell({
    ...(options.shell ?? {}),
    ...(options.persist === undefined ? {} : { persist: options.persist }),
  });
  const event = snapshot(
    "s1",
    options.status ?? { type: "idle" },
    options.session ?? {},
  ) as CoreEvent & {
    snapshot: { state: Record<string, unknown> };
  };
  event.snapshot.state["history"] = history;
  event.snapshot.state["turns"] = turns;
  const change = (c: Record<string, unknown>) => {
    act(() => {
      rig.client.emit({
        type: "change",
        change: { sessionId: "s1", ...c },
      } as unknown as CoreEvent);
    });
  };
  act(() => {
    rig.client.emit(event);
    rig.app.shell.actions.selectSession("s1");
  });
  if (options.openCallbackIds !== undefined)
    change({
      kind: "callbacksChanged",
      openCallbackIds: options.openCallbackIds,
    });
  const settle = async (ms = 20) => {
    await act(async () => {
      await new Promise((r) => setTimeout(r, ms));
    });
  };
  await settle();
  // The composer is a lazy chunk: wait for it to be mounted so no test races it.
  await rig.composer();
  return {
    ...rig,
    patch: (entryId, ops) => {
      change({ kind: "entryPatched", entryId, patch: ops });
    },
    upsert: (entry) => {
      change({ kind: "entryUpserted", entry, replaced: false });
    },
    openCallbacks: (ids) => {
      change({ kind: "callbacksChanged", openCallbackIds: ids });
    },
    settle,
  };
}

/** The row element for a row key. */
export const rowEl = (key: string): HTMLElement | null =>
  document.querySelector<HTMLElement>(`[data-row-key="${key}"]`);
