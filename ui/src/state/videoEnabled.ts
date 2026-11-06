/** Release video rows are visible; development fixtures can explicitly exercise the disabled-core view. */
export const VIDEO_ENABLED = import.meta.env.PROD || import.meta.env["VITE_VIDEO"] !== "0";
