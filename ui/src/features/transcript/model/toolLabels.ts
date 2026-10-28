/** Plain labels for Vibe's built-in actions; keeps custom tool identities and command contents intact.
 * Depends on no UI state. Called by permission, transcript and saved-rule wording.
 */
interface ToolWords {
  name: string;
  permission: string;
}

/** Known built-in action, or null for a third-party tool whose identity must stay visible. */
export function builtinTool(tool: string): ToolWords | null {
  if (tool.startsWith("mcp_vw.")) {
    const action = APP_ACTIONS[tool.slice(7)];
    if (action !== undefined) return { name: action, permission: "this action" };
  }
  const name = tool.replace(/^(?:file_system|ui)\./, "");
  switch (name) {
    case "bash":
    case "shell":
      return { name: "Run commands", permission: "running commands" };
    case "read_file":
      return { name: "Read files", permission: "reading files" };
    case "write_file":
      return { name: "Write files", permission: "creating and changing files" };
    case "edit":
    case "search_replace":
      return { name: "Change files", permission: "changing files" };
    case "grep":
      return { name: "Search file contents", permission: "searching file contents" };
    case "glob":
      return { name: "Find files", permission: "finding files" };
    case "list_directory":
      return { name: "List folders", permission: "listing folders" };
    case "web.fetch":
    case "web_fetch":
      return { name: "Read web pages", permission: "reading web pages" };
    case "web.search":
    case "web_search":
      return { name: "Search the web", permission: "searching the web" };
    case "skill":
    case "skill.read":
    case "read_skill":
      return { name: "Read skill instructions", permission: "reading skill instructions" };
    case "task.create":
    case "task.update":
    case "task.list":
    case "task.get":
      return { name: "Manage tasks", permission: "managing tasks" };
    case "todo":
      return { name: "Update the task list", permission: "updating the task list" };
    case "task":
    case "agents.spawn":
    case "subagent.spawn":
    case "agent":
    case "agent.run":
      return { name: "Run a subagent", permission: "running a subagent" };
    case "self.wait":
    case "wait":
    case "self.sleep":
      return { name: "Wait", permission: "waiting" };
    case "agents.list_configs":
      return { name: "List agent configurations", permission: "listing agent configurations" };
    case "agents.list":
    case "subagent.list":
      return { name: "List subagents", permission: "listing subagents" };
    case "agents.send_message":
    case "subagent.send_message":
      return { name: "Message subagents", permission: "sending messages to subagents" };
    case "agents.interrupt":
    case "subagent.interrupt":
      return { name: "Interrupt subagents", permission: "interrupting subagents" };
    case "subagent.stop":
      return { name: "Stop subagents", permission: "stopping subagents" };
    case "subagent.wait":
    case "agent.wait":
      return { name: "Wait for subagents", permission: "waiting for subagents" };
    case "ask_user_question":
    case "ask_user":
    case "user_input":
      return { name: "Ask a question", permission: "asking a question" };
    case "exit_plan_mode":
      return { name: "Finish a plan", permission: "finishing a plan" };
    default:
      return null;
  }
}

/** Replace only a built-in's label or display prefix; filenames and exact commands remain unchanged. */
export function toolText(tool: string, text: string, literal: string | null = null): string {
  if (literal !== null && text === literal) return text;
  const known = builtinTool(tool);
  if (known === null) return text;
  for (const name of [tool, tool.replace(/^(?:file_system|ui)\./, "")]) {
    if (text === name || text === `Approve ${name}` || text === `Allow ${name}?`) return known.name;
    if (text.startsWith(`${name}: `)) return text.slice(name.length + 2);
  }
  return text;
}

const APP_ACTIONS: Readonly<Record<string, string>> = {
  document_read: "Read an attached document",
  propose_protected_change: "Propose a change to Vibe's folder",
  protected_change_status: "Read the change outcome",
  read_aloud: "Read aloud",
  processes_list: "List running programs",
  port_lease: "Read this session's ports",
  process_start: "Start a background command",
  process_stop: "Stop a program",
  process_restart: "Restart a program",
  use_your_chrome: "Use your Chrome",
  chrome_outcome: "Read the Chrome outcome",
  use_headless_browser: "Use the background browser",
  secret_list: "List project secrets",
  secret_run: "Run a command with secrets",
  secret_run_status: "Read the command outcome",
  panel_read: "Read an open editor",
  preview_list: "List this session's servers",
  preview_open: "Open a preview",
  preview_screenshot: "Capture a preview",
  preview_console: "Read the preview console",
  preview_close: "Close a preview",
  browser_tabs: "List browser tabs",
  browser_open: "Open a browser page",
  browser_read: "Read a browser page",
  browser_act: "Use a browser page",
  browser_screenshot: "Capture a browser page",
  browser_close: "Close a browser page",
  browser_fill_login: "Fill a saved login",
  browser_check_view: "Check the page layout",
  browser_capture_secret: "Save a selected secret",
  chrome_tool_list: "List Chrome actions",
  browser_capture: "Capture a browser page",
  chrome_status: "Read Chrome status",
  chrome_task: "Use your Chrome",
  chrome_execute: "Use your Chrome",
};
