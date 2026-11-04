/** Video review is available only in explicitly enabled development builds and fixtures. */
export const VIDEO_ENABLED = !import.meta.env.PROD && import.meta.env["VITE_VIDEO"] === "1";
