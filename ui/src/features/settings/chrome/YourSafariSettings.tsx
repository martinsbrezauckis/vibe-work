/** Optional Safari entry uses the same browser card; the core view keeps it absent when the module is off. */
import { YourChromeSettings } from "./YourChromeSettings";

/** The Safari browser card: `YourChromeSettings` for Safari. */
export function YourSafariSettings() {
  return <YourChromeSettings browser="Safari" />;
}
