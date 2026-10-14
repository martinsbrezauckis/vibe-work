/**
 * The host "Open release page" goes to, shown beside that button in the update notice and in Settings. The user sees where the system browser will open before clicking.
 * Depends on: state/updates (`releasePageHost`). Called by: UpdateNotice, UpdateSettings.
 * Invariant: the link itself is built by the host from the official repository only
 * (`vw_host::update_flow::release_page_for`); this only displays its host.
 */
import { releasePageHost } from "../../state/updates";

/** The release page's host as quiet text, or nothing when there is no usable link. */
export function ReleaseHost({ url }: { url: string | null }) {
  const host = releasePageHost(url);
  if (host === null) return null;
  return (
    <span className="d update-link-host" data-testid="release-page-host">
      {host}
    </span>
  );
}
