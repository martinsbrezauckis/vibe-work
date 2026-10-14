/**
 * What the Connectors page knows about connectors by itself: the categories, a recognisable name, a category and one plain line for each connector Vibe's catalog
 * listed for the account (16 aliases, `connector_catalog/read`), the vendors whose MCP servers only
 * approved clients may use, and readable labels for connector
 * tool steps in the transcript.
 * Depends on: nothing. Called by: model/view, model/url, the connector views, the transcript (`entries.ts`).
 * Invariants: no logos; names use generic category icons; an alias not listed here still shows, under
 * "Other", with Vibe's own display name; the tool list a tile shows is always Vibe's, never ours.
 */

/** A category id, including `other` for unknown connectors. */
export type CategoryId =
  "code" | "issues" | "docs" | "comms" | "mail" | "files" | "data" | "pay" | "search" | "other";

/** Categories in page order with their headings. */
export const CATEGORIES: readonly { id: CategoryId; label: string }[] = [
  { id: "code", label: "Code hosting & CI" },
  { id: "issues", label: "Issues & project management" },
  { id: "docs", label: "Docs & knowledge" },
  { id: "comms", label: "Communication" },
  { id: "mail", label: "Email & calendar" },
  { id: "files", label: "Files & storage" },
  { id: "data", label: "Data & databases" },
  { id: "pay", label: "Payments & commerce" },
  { id: "search", label: "Search & research" },
  { id: "other", label: "Other" },
];

/** Our facts about one Mistral-hosted connector. */
export interface ConnectorMeta {
  name: string;
  category: CategoryId;
  /** One plain line: what it is for. */
  line: string;
  /** Vibe marks it experimental (shown only when Vibe's catalog lists it; Vibe Work never switches it on). */
  experimental?: boolean;
}

/** Known aliases (the catalog). */
const KNOWN: Readonly<Record<string, ConnectorMeta>> = {
  web_search: {
    name: "Web search",
    category: "search",
    line: "Searches the web and the news; answers cite their sources.",
  },
  github_app: { name: "GitHub", category: "code", line: "Repositories, issues and pull requests." },
  linear: { name: "Linear", category: "issues", line: "Issues, projects and cycles." },
  atlassian: { name: "Atlassian", category: "issues", line: "Jira issues and Confluence pages." },
  notion: { name: "Notion", category: "docs", line: "Pages and databases." },
  box: { name: "Box", category: "docs", line: "Files and folders in Box." },
  slack: { name: "Slack", category: "comms", line: "Channels, threads and messages." },
  gmail: { name: "Gmail", category: "mail", line: "Your Gmail mail." },
  google_calendar: { name: "Google Calendar", category: "mail", line: "Events and free time." },
  outlook: { name: "Outlook mail", category: "mail", line: "Your Outlook mail." },
  outlook_calendar: { name: "Outlook Calendar", category: "mail", line: "Events and free time." },
  google_drive_mcp: {
    name: "Google Drive",
    category: "files",
    line: "Docs, Sheets and files in Drive.",
  },
  sharepoint_online: {
    name: "SharePoint Online",
    category: "files",
    line: "Sites, lists and documents.",
  },
  sharepoint_mcp: {
    name: "SharePoint Search",
    category: "files",
    line: "Search across SharePoint.",
  },
  bigquery: { name: "BigQuery", category: "data", line: "Datasets and SQL queries." },
  stripe: { name: "Stripe", category: "pay", line: "Customers, payments and subscriptions." },
  // Vibe 2.26.1: built in, listed only when `experimental_enable_document_library_connector` is on.
  document_library: {
    name: "Document Library",
    category: "docs",
    line: "Search, list, open and fetch documents from your Mistral document libraries.",
    experimental: true,
  },
};

/** Vibe's display name, made readable when it is only the id (`outlook_calendar` -> "Outlook calendar"). */
function readable(displayName: string): string {
  if (!/^[a-z0-9_]+$/.test(displayName)) return displayName;
  const words = displayName
    .replace(/_mcp$/, "")
    .split("_")
    .filter((w) => w !== "");
  const text = words.join(" ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** Our facts for `alias`, or a neutral entry built from Vibe's display name. */
export function metaOf(alias: string, displayName: string): ConnectorMeta {
  return (
    KNOWN[alias] ?? {
      name: readable(displayName),
      category: "other",
      line: "A connector from your Mistral account.",
    }
  );
}

/** A vendor whose remote MCP server only approved clients may use. */
export interface GatedVendor {
  vendor: string;
  /** Host names that belong to it. */
  hosts: readonly string[];
  /** The vendor's own rule, quoted. */
  rule: string;
  source: string;
}

/** Vendors known to gate their MCP server on the client. */
export const GATED: readonly GatedVendor[] = [
  {
    vendor: "Figma",
    hosts: ["mcp.figma.com"],
    rule: "Only clients listed in the Figma MCP Catalog can connect to the Figma MCP Server.",
    source: "developers.figma.com/docs/figma-mcp-server",
  },
  {
    vendor: "Vercel",
    hosts: ["mcp.vercel.com"],
    rule: "Vercel MCP only supports AI clients that have been reviewed and approved by Vercel.",
    source: "vercel.com/docs/mcp/vercel-mcp",
  },
  {
    vendor: "Slack",
    hosts: ["mcp.slack.com"],
    rule: "Only directory-published apps or internal apps may use MCP.",
    source: "docs.slack.dev/ai/slack-mcp-server",
  },
];

/** The gated vendor that `host` belongs to, if any. */
export function gatedVendor(host: string): GatedVendor | null {
  const h = host.toLowerCase();
  return GATED.find((g) => g.hosts.some((x) => h === x || h.endsWith(`.${x}`))) ?? null;
}

/**
 * A readable label for a connector tool step (`connector_linear.list_issues` -> "Linear: list issues"),
 * or `null` for any other tool. Unified Vibe 2.26.0 labels these steps with the raw tool name, so the transcript uses this instead.
 */
export function connectorToolLabel(toolName: string): string | null {
  const match = /^connector_([a-z0-9_]+?)\.([A-Za-z0-9_-]+)$/.exec(toolName);
  if (match === null) return null;
  const alias = match[1] ?? "";
  const tool = (match[2] ?? "").replace(/[_-]+/g, " ").trim();
  return `${metaOf(alias, alias).name}: ${tool}`;
}
