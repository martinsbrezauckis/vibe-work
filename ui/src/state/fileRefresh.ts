/** Workspace-owned event batching and visible-file fallback. Readers retain drafts and stale-save fingerprints. */
import type { CoreEvent } from "../protocol/core";
import type { CoreController } from "./coreStore";
import { POLL_MS, tabOf, type FileReader } from "./files";

interface Pending {
  all: boolean;
  files: Set<string>;
  viewer: boolean;
}

export function createFileRefresh(
  core: CoreController,
  sessions: () => readonly string[],
  readers: (id: string) => readonly FileReader[],
) {
  const pending = new Map<string, Pending>();
  const busy = new Set<string>();
  const visible = new Map<string, { id: string; file: string; count: number }>();
  let timer: ReturnType<typeof setTimeout> | null = null;
  let fallback: ReturnType<typeof setInterval> | null = null;
  let stopped = false;
  const known = (id: string) => !stopped && sessions().includes(id);
  const queue = () => {
    if (timer !== null || stopped) return;
    timer = setTimeout(() => {
      timer = null;
      for (const id of pending.keys()) if (!busy.has(id)) void refresh(id);
    }, 50);
  };
  const refresh = async (id: string) => {
    const work = pending.get(id);
    pending.delete(id);
    if (!work || !known(id)) return;
    busy.add(id);
    try {
      const documents = readers(id);
      await Promise.all([
        ...documents
          .filter((reader) => {
            const file = tabOf(reader.store.get(), id).shown?.file;
            return file !== undefined && (work.all || work.files.has(file));
          })
          .map((reader) => reader.poll(id)),
        ...(work.viewer
          ? [
              core
                .run({ type: "files", request: { op: "viewer", sessionId: id } })
                .then((result) => {
                  if (known(id) && result.type === "files" && result.result.kind === "viewer")
                    for (const reader of readers(id)) reader.setViewer(id, result.result.viewer);
                })
                .catch(() => undefined),
            ]
          : []),
      ]);
    } finally {
      busy.delete(id);
      // A change during a read must get a fresh check after it, never an overlapping stat.
      if (pending.has(id)) queue();
    }
  };
  const schedule = (id: string, file?: string, viewer = false) => {
    if (!known(id)) return;
    const work = pending.get(id) ?? { all: false, files: new Set<string>(), viewer: false };
    if (file !== undefined) work.files.add(file);
    else if (!viewer) work.all = true;
    work.viewer ||= viewer;
    pending.set(id, work);
    queue();
  };
  const invalidate = (id: string) => {
    for (const reader of readers(id)) reader.invalidate(id);
  };
  return {
    schedule,
    event(event: CoreEvent) {
      if (event.type === "changesUpdated") {
        if (known(event.sessionId)) invalidate(event.sessionId);
        schedule(event.sessionId);
      } else if (event.type === "filesChanged" && event.sessionId !== null) {
        schedule(event.sessionId, undefined, true);
      } else if (event.type === "lagged") {
        for (const id of sessions()) {
          invalidate(id);
          schedule(id);
          schedule(id, undefined, true);
        }
      }
    },
    watch(id: string, file: string) {
      const key = `${id}\u0000${file}`;
      const entry = visible.get(key) ?? { id, file, count: 0 };
      entry.count++;
      visible.set(key, entry);
      fallback ??= setInterval(() => {
        for (const { id, file } of visible.values()) {
          // A slow read already covers the fallback; only real events require another check.
          if (!busy.has(id)) schedule(id, file);
        }
      }, POLL_MS);
      return () => {
        entry.count--;
        if (entry.count === 0) visible.delete(key);
        if (visible.size === 0 && fallback !== null) {
          clearInterval(fallback);
          fallback = null;
        }
      };
    },
    forget(ids: readonly string[]) {
      for (const id of ids) pending.delete(id);
    },
    stop() {
      stopped = true;
      if (timer !== null) clearTimeout(timer);
      if (fallback !== null) clearInterval(fallback);
      pending.clear();
      visible.clear();
    },
  };
}
