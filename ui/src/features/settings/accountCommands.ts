/**
 * Typed account-browser commands for Settings; account status is a cookie-presence hint only.
 * Depends on: coreStore and generated browser types. Called by: SignedInAccounts.
 * Invariant: this module never receives cookies, credentials or account identities.
 */
import type { BrowserRequest, BrowserResult, Provider } from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";

/** Fixed provider labels in display order. */
export const ACCOUNT_PROVIDERS: readonly { provider: Provider; label: string }[] = [
  { provider: "google", label: "Google" },
  { provider: "gitHub", label: "GitHub" },
  { provider: "microsoft", label: "Microsoft" },
  { provider: "apple", label: "Apple" },
];

/** Sends one browser-account command and refuses an unexpected result type. */
export async function accountCommand(
  core: CoreController,
  request: BrowserRequest,
): Promise<BrowserResult> {
  const result = await core.run({ type: "browser", request });
  if (result.type !== "browser") throw new Error("Could not read the browser account state.");
  return result.result;
}
