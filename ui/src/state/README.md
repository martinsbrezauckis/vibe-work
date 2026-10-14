# state

Stores fed by the Rust state stream (snapshots plus JSON-Patch). Components read from here; business
logic belongs in this folder or in `features/*/model.ts`, not in components.

Files: `createStore.ts` (the tiny store and `useStore`), `coreModel.ts` + `sessionChanges.ts` +
`jsonPatch.ts` (the core model and the reducer for `CoreEvent`s, same patch rules as `vw-session`),
`sessionSummary.ts` (rows and badges), `coreStore.ts` (the controller: subscribe, read, send commands),
`shellState.ts` + `panelLayout.ts` + `shellStore.ts` + `persist.ts` (panel, list, theme, widths),
registries other features plug into: `shortcuts.ts`, `commands.ts` (palette), `panelTabs.ts` (tab bodies),
`uiControl.ts` (Debug API `ui/*` routes), `toastStore.ts` + `notifications.ts`.

Bell history admits only an explicit `bellKind`: `approval`, `finished`, `failure`, or `security`.
Severity alone never retains an entry. Routine confirmation belongs beside its control, or needs
no message when the resulting state is visible. Waiting callbacks are deduplicated by session and
callback ID and forgotten when answered/closed. Finished turns follow the notification level and
attention rule in `osNotify.ts`; the OS transport still applies that rule to every banner.
