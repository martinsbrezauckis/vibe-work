/**
 * The knowledge base for Settings: the user's named places (folders, cloud drives, Mistral Libraries) the agent
 * looks at first. Reads on mount; Add, Rename and Remove go through the core's `knowledge` command, which checks and
 * saves them. The user's Mistral Libraries are listed only when asked (one `libraries list` call with the saved key).
 * Depends on: protocol/core types, coreStore. Called by: KnowledgeSettings.
 * Invariants: a refused change keeps the confirmed list and shows the core's plain reason until the next change,
 * an edit in the form or the form's closing; nothing here reads, copies or indexes a place.
 */
import { useCallback, useEffect, useState } from "react";
import type {
  KnowledgeKind,
  KnowledgePlace,
  KnowledgeRequest,
  LibraryView,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";

/** What Add sends. */
export interface NewPlace {
  name: string;
  kind: KnowledgeKind;
  location: string;
  label?: string;
  note?: string;
}

/** Read and change the knowledge base; `libraries` stays `null` until `loadLibraries` ran. */
export function useKnowledge(core: CoreController | null) {
  const [places, setPlaces] = useState<KnowledgePlace[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [libraries, setLibraries] = useState<LibraryView[] | null>(null);
  const [librariesError, setLibrariesError] = useState<string | null>(null);

  const send = useCallback(
    async (request: KnowledgeRequest): Promise<boolean> => {
      if (core === null) return false;
      setBusy(true);
      setError(null);
      try {
        const result = await core.run({ type: "knowledge", request });
        if (result.type === "knowledge") setPlaces(result.view.places);
        return true;
      } catch (thrown) {
        setError(toCallError(thrown).error.message);
        return false;
      } finally {
        setBusy(false);
      }
    },
    [core],
  );

  useEffect(() => {
    if (core === null) return;
    let live = true;
    void core
      .run({ type: "knowledge", request: { op: "list" } })
      .then((result) => {
        if (live && result.type === "knowledge") setPlaces(result.view.places);
      })
      .catch((thrown: unknown) => {
        if (live) setError(toCallError(thrown).error.message);
      });
    return () => {
      live = false;
    };
  }, [core]);

  const loadLibraries = async () => {
    if (core === null) return;
    setLibrariesError(null);
    try {
      const result = await core.run({ type: "libraries", request: { op: "list" } });
      if (result.type === "libraries" && result.result.kind === "list")
        setLibraries(result.result.libraries);
    } catch (thrown) {
      setLibrariesError(toCallError(thrown).error.message);
    }
  };

  return {
    places,
    busy,
    error,
    libraries,
    librariesError,
    loadLibraries,
    /** Forgets the shown refusal (the person edited the form it was about, or closed it). */
    clearError: () => {
      setError(null);
    },
    add: (place: NewPlace) => send({ op: "add", ...place }),
    rename: (id: string, name: string) => send({ op: "rename", id, name }),
    remove: (id: string) => send({ op: "remove", id }),
  };
}
