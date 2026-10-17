/**
 * Links to Mistral's own pages, by Settings category. Most account settings of a Pro user live on Mistral's websites, where the user is signed in; Vibe Work only
 * links there. Every link opens in the system browser (the core's `openLink`), never inside the app: the app never
 * holds a Mistral web session.
 * Paths: only the documented ones lists (docs.mistral.ai, help.mistral.ai, Vibe 2.26.0's own upgrade action).
 * Profile and sign-in/security links open the site's home and explain where to click. API training
 * opt-out and zero data retention open the admin Privacy page. Web Instructions and Knowledge are omitted
 * because those website settings do not apply to Vibe Work.
 * Rows with `webOnly` explain a web setting that has no effect in Vibe Work.
 * Depends on: nothing. Called by: MistralLinks (the rows), catalog (search), settingsRoutes (Debug API), tests.
 * Invariants: every URL is https on a mistral.ai host (a test checks it); ids are unique and are the rows'
 * `data-setting` anchors.
 */

/** One link row. */
export interface MistralLink {
  id: string;
  category: string;
  label: string;
  /** One line: what the page does, or where to click, or that it does not apply here. */
  note: string;
  url: string;
  /** A web setting that Vibe Work does not use (the note says so). */
  webOnly?: boolean;
  /** Extra search words. */
  keywords: string;
}

const CHAT = "https://chat.mistral.ai";
const ADMIN = "https://admin.mistral.ai";
/** The admin console's Privacy page (the bare console opens Organization). */
const ADMIN_PRIVACY = `${ADMIN}/plateforme/privacy`;

/** The Vibe CLI key page (Code › Vibe CLI): a plan key, using the plan's included usage. */
export const VIBE_KEY_URL = `${CHAT}/code/extensions?focus=key`;

/** Every link, in page order. */
export const MISTRAL_LINKS: readonly MistralLink[] = [
  // Account
  {
    id: "link-vibe-key",
    category: "account",
    label: "Get or revoke a Vibe key",
    note: "Code › Vibe CLI. Create or revoke the key for your plan.",
    url: VIBE_KEY_URL,
    keywords: "api key create new plan cli code extensions",
  },
  {
    id: "link-api-keys",
    category: "account",
    label: "API keys",
    note: "Your Studio API keys: list, expiry, rotate, delete.",
    url: "https://console.mistral.ai/home?profile_dialog=api-keys",
    keywords: "studio keys revoke rotate expire",
  },
  {
    id: "link-profile",
    category: "account",
    label: "Profile and email",
    note: "On chat.mistral.ai: your organization's name at the bottom left, then Profile.",
    url: CHAT,
    keywords: "name email profile",
  },
  {
    id: "link-security",
    category: "account",
    label: "Password and two-factor sign-in",
    note: "On chat.mistral.ai: your organization's name at the bottom left, then Security & Access.",
    url: CHAT,
    keywords: "password 2fa mfa two factor authenticator security sign in",
  },
  {
    id: "link-workspace",
    category: "account",
    label: "Workspace switcher",
    note: "Your key belongs to one Workspace. To work in another here, add a key from that Workspace.",
    url: CHAT,
    webOnly: true,
    keywords: "workspace organization switch",
  },
  // Usage and plan
  {
    id: "link-monthly-usage",
    category: "usage",
    label: "Monthly usage",
    note: "Mistral shows how much of this month's included usage is used on its website, not to apps.",
    url: "https://console.mistral.ai/codestral/cli",
    keywords: "quota percent budget month included usage left",
  },
  {
    id: "link-subscription",
    category: "usage",
    label: "Plan and included usage",
    note: "Manage, upgrade or cancel your plan on the website.",
    url: `${ADMIN}/subscription`,
    keywords: "subscription plan upgrade cancel pay as you go payg pro",
  },
  {
    id: "link-billing",
    category: "usage",
    label: "Billing, spending limit and invoices",
    note: "Manage billing details and spending limits on the website.",
    url: `${ADMIN}/organization/billing`,
    keywords: "billing payment invoice vat credits spending limit",
  },
  {
    id: "link-usage-dashboard",
    category: "usage",
    label: "Usage dashboard",
    note: "Usage by day, model and Workspace on the website.",
    url: `${ADMIN}/organization/usage`,
    keywords: "usage dashboard cost daily",
  },
  {
    id: "link-rate-limits",
    category: "usage",
    label: "Rate limits",
    note: "The full table: requests per second, tokens per minute and per month, by model.",
    url: `${ADMIN}/plateforme/limits`,
    keywords: "rate limits tokens per minute requests",
  },
  // Privacy and data
  {
    id: "link-vibe-training",
    category: "privacy",
    label: "Training opt-out for Vibe",
    note: "Whether your Vibe sessions may train Mistral's models. Separate from the API opt-out below.",
    url: `${ADMIN}/vibe/privacy`,
    keywords: "training opt out train models data",
  },
  {
    id: "link-api-training",
    category: "privacy",
    label: "Training opt-out for the API",
    note: "Opens Privacy in the admin console: Anonymous improvement data. Turning off one does not turn off the other.",
    url: ADMIN_PRIVACY,
    keywords: "training opt out api anonymous improvement data",
  },
  {
    id: "link-zdr",
    category: "privacy",
    label: "Zero data retention",
    note: "Opens Privacy in the admin console. This app talks to Mistral like the Vibe CLI, so it applies if your organization has it.",
    url: ADMIN_PRIVACY,
    keywords: "zero data retention zdr",
  },
  {
    id: "link-export",
    category: "privacy",
    label: "Export my data",
    note: "Ask Mistral for a copy of your account's data.",
    url: `${ADMIN}/account/export`,
    keywords: "export download data gdpr",
  },
  {
    id: "link-delete-account",
    category: "privacy",
    label: "Delete my account",
    note: "On the account page, under Danger Zone.",
    url: `${ADMIN}/account`,
    keywords: "delete account close remove",
  },
];

/** The links of one category, in page order. */
export function linksOf(category: string): readonly MistralLink[] {
  return MISTRAL_LINKS.filter((l) => l.category === category);
}
