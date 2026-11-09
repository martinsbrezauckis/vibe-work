/** Browser approval descriptions from known action IDs and key arguments, never agent-supplied summaries.
 * Depends on vocabulary and transcript JSON accessors; called by the AskCard model, card and bell. Raw input stays below.
 */
import { TEXT, wording } from "../../vocabulary";
import type { JsonValue } from "../../protocol/core";
import { approvalEffect } from "../transcript/model/callbacks";
import { type Obj, arr, detailOf, obj, str } from "../transcript/model/entries";
import { BROWSER_ACTIONS, BUILTIN_ACTIONS, SESSION_ACTIONS } from "./browserApprovalActions";

function toolOf(entry: Obj): string {
  return str(approvalEffect(entry)["toolName"]) ?? "";
}

/** Browser identity projected by the core from this action's helper session binding. */
export function browserApprovalName(entry: Obj): string | null {
  const tool = toolOf(entry);
  if (/^mcp_vw\.browser_(read|act)$/.test(tool)) return "the built-in browser";
  if (!/^mcp_vw\.chrome_(execute|session|task|status)$/.test(tool)) return null;
  const browser = str(entry["vwBrowser"]);
  return browser === TEXT.chromeName ||
    browser === TEXT.edgeName ||
    browser === TEXT.safariName ||
    browser === TEXT.firefoxName
    ? browser
    : "your browser";
}

/** URL parsing displays the destination host, excluding credentials, path, query and fragment. */
function host(value: unknown): string | null {
  if (typeof value !== "string") return null;
  try {
    const url = new URL(value);
    return ["http:", "https:"].includes(url.protocol) ? url.host : null;
  } catch {
    return null;
  }
}

/** Only URLs that determine this action's destination can name its host; a stray URL on typing is ignored. */
function pageOf(action: string, input: Obj, args: Obj, session: boolean): string {
  const target = session && action === "open" ? obj(input["target"])?.["url"] : undefined;
  const address = ["page.navigate", "tabs.create", "files.download"].includes(action)
    ? args["url"]
    : target;
  return host(address) ?? "this page";
}

/** Only a short, single-line key name is interpolated; typed text and scripts remain in the raw payload. */
function keyOf(args: Obj): string {
  return (
    (str(args["key"]) ?? "a key")
      .replace(/[\p{Cc}\p{Cf}]/gu, "")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 40) || "a key"
  );
}

/** Page-provided labels stay a short quotation inside the action, never an instruction or extra card note. */
function clickTarget(value: JsonValue | undefined): string {
  const target = obj(value);
  const name = str(target?.["name"]);
  if (name === null) return "an unknown target";
  const clean = name
    .replace(/[\p{Cc}\p{Cf}]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (clean === "") return "an unknown target";
  const role = str(target?.["role"]);
  const kind =
    role !== null && ["button", "link", "checkbox", "radio", "menuitem", "tab"].includes(role)
      ? role
      : "element";
  const letters = Array.from(clean);
  const quoted =
    letters.slice(0, 80).join("").replace(/[“”]/g, '"') + (letters.length > 80 ? "…" : "");
  return `${kind} “${quoted}”`;
}

function describe(
  action: string,
  input: Obj,
  args: Obj,
  browser: string,
  target: JsonValue | undefined = null,
  session = false,
): string {
  const table = session ? SESSION_ACTIONS : BROWSER_ACTIONS;
  const template = Object.hasOwn(table, action) ? table[action] : undefined;
  if (template === undefined)
    return `Unrecognized browser action in ${browser}; review the arguments`;
  const limit = args["limit"];
  const page = pageOf(action, input, args, session);
  if (action === "tabs.create" && page === "this page") return `Open a new tab in ${browser}`;
  let line = wording(template, {
    page,
    in: ` in ${browser}`,
    your: browser.replace(/^Your /, "your "),
    limit:
      typeof limit === "number" && Number.isSafeInteger(limit) && limit > 0
        ? ` (up to ${String(limit)} entries)`
        : "",
    submit: args["submit"] === true ? " and submit it" : "",
    key: keyOf(args),
  });
  if (
    [
      "page.click",
      "page.double_click",
      "page.triple_click",
      "page.right_click",
      "page.middle_click",
    ].includes(action)
  )
    line = line.replace("an element", clickTarget(target));
  if (action === "page.commit") line = line.replace("a form", `with ${clickTarget(target)}`);
  return action === "network.headers" && args["unredacted"] === true
    ? `${line}, including unredacted values`
    : line;
}

/** One plain summary. A batch names every step, including sensitive actions at its end. */
export function browserApprovalSummary(entry: Obj): string | null {
  const browser = browserApprovalName(entry);
  if (browser === null) return null;
  const tool = toolOf(entry);
  const input = obj(approvalEffect(entry)["input"]) ?? {};
  const args = tool === "mcp_vw.browser_act" ? input : (obj(input["args"]) ?? input);
  if (tool === "mcp_vw.chrome_status") return `Read browser connection details in ${browser}`;
  if (tool === "mcp_vw.browser_read") {
    const tab = input["tab"];
    const target =
      tab === "new"
        ? "Open a new tab and read its page"
        : tab
          ? "Read the selected tab's page"
          : "Read the current page";
    return `${target}${input["screenshot"] === true ? " and take a screenshot" : ""} in ${browser}`;
  }
  if (tool === "mcp_vw.chrome_execute" && Array.isArray(input["steps"])) {
    const steps = arr(input["steps"]);
    if (steps.length === 0 || steps.length > 16)
      return `Invalid browser batch in ${browser}; review the arguments`;
    return steps
      .map((raw) => {
        const step = obj(raw) ?? {};
        return describe(str(step["action"]) ?? "", input, obj(step["args"]) ?? {}, browser);
      })
      .join("; ");
  }
  let action = str(input["action"]) ?? "";
  if (tool === "mcp_vw.browser_act") {
    if (action === "close_tab") return `Close the selected tab in ${browser}`;
    action =
      action === "navigate" && input["tab"] === "new"
        ? "tabs.create"
        : action === "check" && input["checked"] === false
          ? "page.uncheck"
          : ((Object.hasOwn(BUILTIN_ACTIONS, action) ? BUILTIN_ACTIONS[action] : undefined) ?? "");
  }
  const discovered = str(detailOf(entry)["vwBrowserAction"]);
  if (
    entry["vwToolAsk"] === true &&
    discovered !== null &&
    ["page.commit", "page.type", "auth.fill", "raw.execute"].includes(discovered)
  )
    action = discovered;
  if (["tabs.close_owned", "tabs.close_user"].includes(action)) {
    const tab = obj(obj(approvalEffect(entry)["display"])?.["tab"]);
    const id = args["tabId"];
    if (tab && tab["tabId"] === id && typeof id === "number" && Number.isSafeInteger(id)) {
      const kind = str(tab["kind"]);
      const clean = (value: JsonValue | undefined) =>
        (str(value) ?? "")
          .replace(/[\p{Cc}\p{Cf}]/gu, "")
          .replace(/\s+/g, " ")
          .trim()
          .slice(0, 120);
      const title = clean(tab["title"]);
      const label =
        title ||
        (kind === "blank tab"
          ? "blank tab"
          : kind === "browser page"
            ? "browser page"
            : clean(tab["url"]) || "tab");
      return `Close ${label} in ${browser}`;
    }
  }
  return describe(
    action,
    input,
    args,
    browser,
    entry["vwClickTarget"],
    /^mcp_vw\.chrome_(session|task)$/.test(tool),
  );
}
