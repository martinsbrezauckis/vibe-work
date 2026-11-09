/** Fixed consent verbs for supported browser actions. Depends on no state; browserApproval fills bounded targets. */
import actions from "../../../../crates/vw-ask/src/browser_actions.json";
import builtin from "../../../../crates/vw-agent-tools/src/browser_actions.json";

export const BROWSER_ACTIONS: Readonly<Record<string, string>> = actions.actions;

/** Built-in browser wire names mapped to the same plain actions. */
export const BUILTIN_ACTIONS: Readonly<Record<string, string>> = builtin.actions;

export const SESSION_ACTIONS: Readonly<Record<string, string>> = {
  open: "Start browser work on {page}{in}",
  get: "Read browser work status{in}",
  list: "List browser work{in}",
  stop: "Stop browser work{in}",
  cancel: "Cancel the current browser action{in}",
  delete: "End browser work{in}",
};
