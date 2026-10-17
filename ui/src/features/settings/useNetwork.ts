/**
 * The state and actions of Settings > Network: read what is saved (the core's own file), edit the form, test the way
 * to Mistral, save (which also writes Vibe, so it needs Vibe running).
 * Depends on: AppContext (toasts, sessionTools), hooks, state/network, vibeSettingRows (LEFTOVER_TEXT). Called by:
 * NetworkSettings.
 * Invariants:
 * - Nothing is written until Save; Save asks the native confirmation in the core, and a decline is told, not shown as
 *   an error. A value the core refuses stays in the form with its sentence under its field.
 * - Editing a field drops its sentence and the test result (both were about the old text).
 * - After a save the form is the core's answer (addresses masked again), never the text that was typed.
 * - A short-lived chat the core could not delete is a notification, like the Subagents switch.
 */
import { useCallback, useEffect, useState } from "react";

import type { NetworkResult, NetworkTestView, NetworkView } from "../../protocol/core";
import {
  FIELD_NAMES,
  type FieldName,
  type Form,
  changedFields,
  changesOf,
  failure,
  formOf,
  networkCall,
} from "../../state/network";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { LEFTOVER_TEXT } from "./vibeSettingRows";

type Busy = "read" | "test" | "save" | null;

const EMPTY: Form = {
  httpsProxy: "",
  httpProxy: "",
  allProxy: "",
  noProxy: "",
  certificateFile: "",
  certificateFolder: "",
};

/** What the screen needs. */
export function useNetwork() {
  const { core, toasts, sessionTools } = useApp();
  const running = useCoreState((s) => s.server.phase === "running");
  const [view, setView] = useState<NetworkView | null>(null);
  const [form, setForm] = useState<Form>(EMPTY);
  const [busy, setBusy] = useState<Busy>(null);
  const [problems, setProblems] = useState<Partial<Record<FieldName, string>>>({});
  const [error, setError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<NetworkTestView | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const leftover = useCallback(
    (id: string | null) => {
      if (id === null) return;
      toasts.push({ kind: "warn", bellKind: "failure", text: LEFTOVER_TEXT, sessionId: id });
      void sessionTools?.refresh();
    },
    [toasts, sessionTools],
  );

  /** Shows a `network` answer that carries the settings. */
  const show = useCallback(
    (result: NetworkResult) => {
      if (result.kind !== "settings") throw new Error("unexpected answer");
      setView(result.view);
      setForm(formOf(result.view));
      leftover(result.leftoverSessionId);
    },
    [leftover],
  );

  /** "Try again": reads the settings once more. */
  const read = useCallback(() => {
    if (core === null) return;
    setBusy("read");
    setError(null);
    networkCall(core, { op: "read" })
      .then(show)
      .catch((thrown: unknown) => {
        setError(failure(thrown).text);
      })
      .finally(() => {
        setBusy(null);
      });
  }, [core, show]);

  // The first read (the effect only starts the call; the answer sets the state). What is shown is the core's own file,
  // so no running Vibe is needed to see it; only saving needs Vibe.
  useEffect(() => {
    if (core === null) return;
    let live = true;
    networkCall(core, { op: "read" })
      .then((result) => {
        if (live) show(result);
      })
      .catch((thrown: unknown) => {
        if (live) setError(failure(thrown).text);
      });
    return () => {
      live = false;
    };
  }, [core, show]);

  const edit = useCallback((name: FieldName, text: string) => {
    setForm((current) => ({ ...current, [name]: text }));
    setProblems((current) => ({ ...current, [name]: undefined }));
    setOutcome(null);
    setNote(null);
  }, []);

  const dirty = view !== null && changedFields(form, view).length > 0;

  const run = useCallback(
    (op: "test" | "save") => {
      if (core === null || view === null) return;
      setBusy(op);
      setError(null);
      setNote(null);
      setProblems({});
      if (op === "test") setOutcome(null);
      networkCall(core, { op, changes: changesOf(form, view) })
        .then((result) => {
          if (result.kind === "refused") {
            const next: Partial<Record<FieldName, string>> = {};
            for (const p of result.problems) {
              const name = FIELD_NAMES.find((n) => n === p.field);
              if (name !== undefined) next[name] = p.message;
            }
            setProblems(next);
          } else if (result.kind === "tested") {
            setOutcome(result.outcome);
          } else {
            setView(result.view);
            setForm(formOf(result.view));
            setNote(
              result.saved
                ? "Saved. It applies the next time Vibe Work starts: close it and open it again."
                : "Nothing to save: these are already the saved settings.",
            );
            leftover(result.leftoverSessionId);
          }
        })
        .catch((thrown: unknown) => {
          const { text, declined } = failure(thrown);
          if (declined) setNote("Not saved: you did not confirm the change.");
          else setError(text);
        })
        .finally(() => {
          setBusy(null);
        });
    },
    [core, view, form, leftover],
  );

  const chooseCertificate = useCallback(() => {
    if (core === null) return;
    core.client
      .pickCertificate()
      .then((path) => {
        if (path !== null) edit("certificateFile", path);
      })
      .catch((thrown: unknown) => {
        setError(failure(thrown).text);
      });
  }, [core, edit]);

  return {
    core,
    running,
    view,
    form,
    busy,
    problems,
    error,
    outcome,
    note,
    dirty,
    edit,
    read,
    chooseCertificate,
    testConnection: () => {
      run("test");
    },
    save: () => {
      run("save");
    },
  };
}
