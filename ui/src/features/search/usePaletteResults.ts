/**
 * Builds global palette rows from local sessions/settings/actions and core content searches.
 * Depends on: AppContext, shell hooks, command registry, settings catalog, coreRows, paletteModel.
 * Called by: CommandPalette. Surviving identities retain their order while typing or receiving newer answers.
 */
import { useMemo, useState } from "react";
import { type PaletteCommand, filterCommands } from "../../state/commands";
import { formatKeys } from "../../state/shortcuts";
import { categoryOf, searchSettings, settingPath } from "../settings/catalog";
import { openHit } from "../settings/openHit";
import { useApp } from "../shell/AppContext";
import { useSessionSummaries } from "../shell/hooks";
import { coreRows } from "./coreRows";
import { paletteQuery, reconcileRows, type ResultGroups } from "./paletteModel";
import { GROUPS, type Result } from "./paletteTypes";
import type { CoreResults } from "./useCoreSearch";

/** Returns fresh row content in stable per-type order; changing query mode starts a new list. */
export function usePaletteResults(query: string, found: CoreResults): ResultGroups {
  const app = useApp();
  const sessions = useSessionSummaries();
  const mode = paletteQuery(query).mode;
  const groups = useMemo((): ResultGroups => {
    const { mode, text } = paletteQuery(query);
    const sessionItems: PaletteCommand[] = sessions
      .filter((s) => !s.archived)
      .map((s) => ({
        id: `session:${s.id}`,
        label: s.title,
        section: "Sessions",
        keywords: [s.preview, s.cwd ?? "", ...s.extraFolders, s.model ?? ""].join(" "),
        hint: s.project,
        run: () => {
          app.shell.actions.closeSettings();
          void app.sessions?.open(s.id);
        },
      }));
    const session: Result[] =
      mode === "action"
        ? []
        : filterCommands(sessionItems, text).map((c) => {
            const s = sessions.find((s) => `session:${s.id}` === c.id);
            return {
              id: c.id,
              kind: "session",
              title: c.label,
              path: [s?.project ?? "", c.label].filter((p) => p !== ""),
              hint: s?.project,
              detail: s?.preview ?? "",
              opens: "Opens the session where you left it",
              run: c.run,
            };
          });
    const setting: Result[] =
      mode === "text"
        ? searchSettings(text).map((e) => ({
            id: `setting:${e.id}`,
            kind: "setting",
            title: e.label,
            path: settingPath(e),
            detail: e.description,
            opens:
              e.opens === "connectors"
                ? "Opens the Tools page"
                : `Opens Settings › ${categoryOf(e.category).label} at this setting`,
            run: () => {
              openHit(app, e);
            },
          }))
        : [];
    const action: Result[] =
      mode === "text"
        ? []
        : filterCommands(app.commands.list(), text).map((c) => ({
            id: c.id,
            kind: "action",
            title: c.label,
            path: ["Actions", ...(c.section === "Actions" ? [] : [c.section])],
            hint: c.keys === undefined ? undefined : formatKeys(c.keys),
            detail: c.keys === undefined ? "" : `Shortcut: ${formatKeys(c.keys)}`,
            opens: "Runs now",
            run: c.run,
          }));
    const { message, file } = mode === "text" ? coreRows(app, found) : { message: [], file: [] };
    return { session, message, file, setting, action };
  }, [app, sessions, query, found]);

  // Guarded render-time adjustment keeps a single render's rows and selection consistent (no effect-frame jump).
  const [snapshot, setSnapshot] = useState({ source: groups, groups, mode });
  if (snapshot.source !== groups || snapshot.mode !== mode) {
    const stable = { ...groups };
    if (mode === snapshot.mode)
      for (const { kind } of GROUPS)
        stable[kind] = reconcileRows(snapshot.groups[kind], groups[kind], (row) => row.id);
    setSnapshot({ source: groups, groups: stable, mode });
    return stable;
  }
  return snapshot.groups;
}
