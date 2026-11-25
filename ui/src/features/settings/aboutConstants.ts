/**
 * Static identity and documentation URLs for About settings and the `ui/settings` route.
 * Depends on: nothing. Called by: AboutSettings, settingsRoutes.
 */

/** The author line. */
export const ABOUT_AUTHOR = "Made by Martins Brezauckis";
/** The contact address, and the only link the contact line may open (`mailto:` to this one address). */
export const CONTACT_EMAIL = "martins.brezauckis@gmail.com";
/** The `mailto:` link for the contact address; also reported in the Debug API's About state. */
export const CONTACT_URL = `mailto:${CONTACT_EMAIL}`;
/** Where the source code lives. */
export const SOURCE_URL = "https://github.com/martinsbrezauckis/vibe-work";
/** The project page (downloads, screenshots) on the ShellX family site. */
export const HOMEPAGE_URL = "https://theshellx.com/vibework";
/** Mistral's own policy; opened by the user, never fetched by this screen. */
export const MISTRAL_DATA_POLICY_URL = "https://legal.mistral.ai/terms/privacy-policy/";
