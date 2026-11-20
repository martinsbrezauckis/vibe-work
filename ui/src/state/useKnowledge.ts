/**
 * The knowledge base for Settings: the user's named places (folders, cloud drives, Mistral Libraries) the agent
 * looks at first. Reads on mount and again on every `knowledgeChanged` event (a change from another window, the
 * agent's client or the Debug API), a lagged event stream and a restarted server; Add, Rename, Edit note and Remove
 * go through the core's `knowledge` command, which checks and saves them. The user's Mistral Libraries are listed
 * only when asked (one `libraries list` call with the saved key); the connectors a cloud drive can be reached
 * through are read only when asked (one `connectors read`).
 * Depends on: protocol/core types, coreStore, the connector view model (status words). Called by:
 * KnowledgeSettings, KnowledgeForms.
 * Invariants: a refused change keeps the confirmed list and shows the core's plain reason until the next change,
 * an edit in the form or the form's closing; nothing here reads, copies or indexes a place; an older read never
 * replaces a newer list.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type {
  ConnectorsView,
  KnowledgeKind,
  KnowledgePlace,
  KnowledgeRequest,
  LibraryView,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import { tileOf } from "../features/connectors/model/view";
import type { CoreController } from "./coreStore";

/** What Add sends. */
export interface NewPlace {
  name: string;
  kind: KnowledgeKind;
  location: string;
  label?: string;
  note?: string;
}

/** One connector a cloud drive can be reached through: its name (what is saved) and its state in words. */
export interface ConnectorChoice {
  name: string;
  status: string;
}

/**
 * The connectors this install has, for the drive picker: Mistral-hosted connectors that are not unavailable (with
 * the Connectors page's status words) and the MCP servers in Vibe Work's own config. A name is offered once.
 */
export function connectorChoices(view: ConnectorsView): ConnectorChoice[] {
  const choices: ConnectorChoice[] = [];
  const seen = new Set<string>();
  const offer = (name: string, status: string) => {
    const key = name.trim().toLowerCase();
    if (key === "" || seen.has(key)) return;
    seen.add(key);
    choices.push({ name: name.trim(), status });
  };
  for (const connector of view.connectors) {
    const tile = tileOf(connector);
    if (tile.status !== "unavailable") offer(tile.meta.name, tile.statusText);
  }
  for (const server of view.servers)
    if (server.configured) offer(server.displayName, server.off ? "Off" : "Your MCP server");
  return choices;
}

/** Read and change the knowledge base; `libraries` stays `null` until `loadLibraries` ran. */
export function useKnowledge(core: CoreController | null) {
  const [places, setPlaces] = useState<KnowledgePlace[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [libraries, setLibraries] = useState<LibraryView[] | null>(null);
  const [librariesError, setLibrariesError] = useState<string | null>(null);

  const [connectors, setConnectors] = useState<ConnectorChoice[] | null>(null);
  const [connectorsFailed, setConnectorsFailed] = useState(false);
  // Every read and change counts; only the newest answer is shown (a re-read the event started while a change
  // was running must not bring back the list from before it).
  const latest = useRef(0);

  const send = useCallback(
    async (request: KnowledgeRequest): Promise<boolean> => {
      if (core === null) return false;
      const read = ++latest.current;
      setBusy(true);
      setError(null);
      try {
        const result = await core.run({ type: "knowledge", request });
        if (result.type === "knowledge" && read === latest.current) setPlaces(result.view.places);
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
    const list = () => {
      const read = ++latest.current;
      void core
        .run({ type: "knowledge", request: { op: "list" } })
        .then((result) => {
          if (live && read === latest.current && result.type === "knowledge")
            setPlaces(result.view.places);
        })
        .catch((thrown: unknown) => {
          if (live && read === latest.current) setError(toCallError(thrown).error.message);
        });
    };
    list();
    const unlisten = core.listen((event) => {
      if (event.type === "knowledgeChanged" || event.type === "lagged") list();
      if (event.type === "server" && event.status.state === "running") list();
    });
    return () => {
      live = false;
      unlisten();
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

  const loadConnectors = async () => {
    if (core === null) return;
    setConnectorsFailed(false);
    try {
      const result = await core.run({ type: "connectors", request: { op: "read" } });
      if (result.type === "connectors" && result.result.kind === "view")
        setConnectors(connectorChoices(result.result.view));
      else setConnectorsFailed(true);
    } catch {
      // The drive form falls back to typing the connector's name.
      setConnectorsFailed(true);
    }
  };

  return {
    places,
    busy,
    error,
    libraries,
    librariesError,
    loadLibraries,
    /** The connectors a cloud drive can be reached through; `null` until `loadConnectors` answered. */
    connectors,
    /** The connectors could not be read (the form offers the typed name). */
    connectorsFailed,
    loadConnectors,
    /** Forgets the shown refusal (the person edited the form it was about, or closed it). */
    clearError: () => {
      setError(null);
    },
    add: (place: NewPlace) => send({ op: "add", ...place }),
    rename: (id: string, name: string) => send({ op: "rename", id, name }),
    /** A new note for the agent; an empty one removes the note. */
    setNote: (id: string, note: string) => send({ op: "setNote", id, note }),
    remove: (id: string) => send({ op: "remove", id }),
  };
}
