/** Optional Safari entry uses the same browser card; the core view keeps it absent when the module is off. */
import { YourChromeSettings } from "./YourChromeSettings";

export function YourSafariSettings() {
  return <YourChromeSettings browser="Safari" />;
}
