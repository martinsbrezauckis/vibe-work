/**
 * Compact browser delegation receipts from the core's structured metadata.
 * Depends on: BrowserState's receipt type. Called by: PreviewTab.
 * Invariant: page text and form values never enter this activity view.
 */
import type { ReactElement } from "react";

import type { DelegationReceipt } from "../../protocol/core";
import { term, TEXT, wording } from "../../vocabulary";

const ACTIONS: Record<DelegationReceipt["act"], string> = {
  closeUserTabs: term("browserPermissions", "closeUserTabs").name,
};

const AUTHORITY: Record<DelegationReceipt["authority"], string> = {
  closeUserTabs: wording(TEXT.browserPermissionAuthority, {
    name: term("browserPermissions", "closeUserTabs").name,
  }),
  confirmedByYou: TEXT.browserConfirmedAuthority,
};

function receiptAuthority(receipt: DelegationReceipt): string {
  return AUTHORITY[receipt.authority];
}

/** Every retained structured receipt for the current session, newest first. */
export function DelegationReceipts({
  receipts,
  expanded = false,
}: {
  receipts: readonly DelegationReceipt[];
  expanded?: boolean;
}): ReactElement | null {
  if (receipts.length === 0) return null;
  return (
    <details className="browser-log browser-receipts" open={expanded}>
      <summary>Browser permissions activity ({String(receipts.length)})</summary>
      <ol className="browser-log-list">
        {[...receipts].reverse().map((receipt, index) => (
          <li
            className="browser-receipt-row"
            key={`${String(receipt.atMs)}-${receipt.act}-${String(index)}`}
            data-outcome={receipt.outcome}
          >
            <time dateTime={new Date(receipt.atMs).toISOString()}>
              {new Date(receipt.atMs).toLocaleString()}
            </time>
            <span>{ACTIONS[receipt.act]}</span>
            <span className="browser-receipt-site">{receipt.site}</span>
            <span>{receipt.outcome}</span>
            <span className="browser-receipt-authority">{receiptAuthority(receipt)}</span>
          </li>
        ))}
      </ol>
    </details>
  );
}
