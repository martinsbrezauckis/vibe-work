/** Release build switch shared by feedback entry points; absent in the default UI build. */
export const FEEDBACK_ENABLED = !import.meta.env.PROD && import.meta.env["VITE_FEEDBACK"] === "1";
