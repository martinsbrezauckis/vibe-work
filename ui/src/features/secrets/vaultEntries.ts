/**
 * Metadata-only Vault list and request state for Settings. Depends on the typed core, shell project
 * selection and key readiness. Called by ProjectSecretsSettings; stored values never arrive here.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type {
  LoginView,
  ProjectSecretView,
  SecretsRequest,
  SecretsResult,
} from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { problem, secrets } from "../../state/secrets";
import { useApp } from "../shell/AppContext";
import { useSessionSummaries, useShellState } from "../shell/hooks";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useGroups } from "../sessions/useGroups";

/** One authoritative metadata listing, also used by the row details. */
export type VaultListing = Extract<SecretsResult, { kind: "list" }>;
/** Editable entry permission words. The core stores defaults and project exceptions. */
export type EntryPermission = NonNullable<ProjectSecretView["permission"]>;
/** A direct user request; callbacks start in the originating click. */
export type VaultAct = (request: SecretsRequest, done: string, saved?: () => void) => void;

/** Selects the current session's folder, or the next session's chosen folder. */
export function useVaultProject(): string | null {
  const selected = useShellState((s) => s.selected);
  const newFolder = useShellState((s) => s.newFolder);
  const sessions = useSessionSummaries();
  return sessions.find((s) => s.id === selected)?.cwd ?? newFolder;
}

/** Reads entry metadata and sends actions, refreshing on both lock and entry events. */
export function useVaultEntries() {
  const { core, keys } = useApp();
  const currentProject = useVaultProject();
  const [chosenProject, setProject] = useState<string | null>(null);
  const project = chosenProject ?? currentProject;
  const sessions = useSessionSummaries();
  const groups = useGroups((s) => s.data);
  const projectFolders = [
    ...new Set([
      ...(project === null ? [] : [project]),
      ...(currentProject === null ? [] : [currentProject]),
      ...(groups?.projects.flatMap((p) => (p.mainFolder === null ? [] : [p.mainFolder])) ?? []),
      ...sessions.flatMap((s) => (s.cwd === null ? [] : [s.cwd])),
    ]),
  ];
  const blocked = useKeyReadiness("vault") !== null;
  const [saved, setList] = useState<{ project: string | null; list: VaultListing } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const latestRead = useRef(0);
  const invalidateReads = useCallback(() => {
    latestRead.current++;
  }, []);
  const handleError = useCallback(
    (thrown: unknown) => {
      if (toCallError(thrown).error.code === "confirmationDeclined") return;
      if (toCallError(thrown).error.serverCode === "passwordRequired") {
        void keys?.load();
        return;
      }
      setError(problem(thrown));
    },
    [keys],
  );
  const refresh = useCallback(() => {
    if (core === null || blocked) return;
    const read = ++latestRead.current;
    void secrets(core, project === null ? { op: "list" } : { op: "listForProject", project })
      .then((result) => {
        if (read === latestRead.current && result.kind === "list") {
          setList({ project, list: result });
          setError(null);
        }
      })
      .catch((error: unknown) => {
        if (read === latestRead.current) handleError(error);
      });
  }, [core, project, blocked, handleError]);
  useEffect(() => {
    if (core === null) return;
    refresh();
    const unlisten = core.listen((event) => {
      if (["secretsChanged", "keyStatusChanged", "lagged"].includes(event.type)) refresh();
    });
    return () => {
      invalidateReads();
      unlisten();
    };
  }, [core, refresh, invalidateReads]);
  const act: VaultAct = (request, done, afterSaved) => {
    if (core === null || blocked || busy) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    // Dispatch immediately; a later async callback must never inherit a user's privileged origin.
    void secrets(core, request)
      .then(() => {
        if (
          request.op === "setEntryPermission" ||
          request.op === "setDefaultPermission" ||
          request.op === "resetEntryPermission"
        ) {
          // Only show a successful write, and discard older reads before updating this project's view.
          invalidateReads();
          const update = <T extends ProjectSecretView | LoginView>(item: T): T => {
            if (item.id !== request.id) return item;
            if (request.op === "setDefaultPermission")
              return {
                ...item,
                defaultPermission: request.permission,
                permission: item.projectPermission ?? request.permission,
              };
            if (request.op === "resetEntryPermission")
              return {
                ...item,
                projectPermission: null,
                permission: item.defaultPermission,
                projectPermissions: item.projectPermissions.filter(
                  (p) => p.project !== request.project,
                ),
              };
            return {
              ...item,
              projectPermission: request.permission,
              permission: request.permission,
              projectPermissions: [
                ...item.projectPermissions.filter((p) => p.project !== request.project),
                { project: request.project, permission: request.permission },
              ],
            };
          };
          setList((saved) =>
            saved === null ||
            (request.op !== "setDefaultPermission" && saved.project !== request.project)
              ? saved
              : {
                  ...saved,
                  list: {
                    ...saved.list,
                    secrets: saved.list.secrets.map(update),
                    logins: saved.list.logins.map(update),
                  },
                },
          );
        }
        setNotice(done);
        afterSaved?.();
        refresh();
      })
      .catch(handleError)
      .finally(() => {
        setBusy(false);
      });
  };
  const chooseFolder = () => {
    if (core === null || busy) return;
    void core.client
      .pickFolder()
      .then((folder) => {
        if (folder !== null) setProject(folder);
      })
      .catch(handleError);
  };
  return {
    core,
    project,
    projectFolders,
    setProject,
    chooseFolder,
    list: blocked || saved?.project !== project ? null : saved.list,
    blocked,
    error,
    notice,
    busy,
    act,
  };
}

/** Full address is retained in the tooltip; the host is the compact list address. */
export function originHost(origin: string): string {
  try {
    return new URL(origin).host || origin;
  } catch {
    return origin;
  }
}
