/**
 * User-only save controls for a page selection or a clicked element. Native code holds the chosen value; this
 * component receives only an opaque handle and display metadata, opens the name prompt, and reports the result.
 * Depends on: browser controller, SaveNameDialog, shared chrome overlay host, app toasts. Called by: PreviewTab.
 */
import { type ReactElement, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { IconButton } from "../../components/IconButton";
import { toCallError } from "../../protocol/client/types";
import { vaultReadiness } from "../../state/keyReadiness";
import { useApp } from "../shell/AppContext";
import { useKeyReadiness } from "../shell/KeyNotice";
import type { Browser, VaultChoice } from "./browser";
import { onContextSave } from "./contextSaveEvents";
import { SaveNameDialog } from "./SaveNameDialog";
import { closeBrowserOverlaysAndWait } from "./browserOverlays";
import { useBrowserPopup } from "./useBrowserPopup";

const RESERVATION = "save-vault-menu";

/** Toolbar entry and name prompt; a native page context click opens the same prompt. */
export function SaveVault({
  browser,
  sessionId,
  tabId,
  disabled,
  compact = false,
  collapsed = false,
  onChoose,
  onNotice,
  visible = true,
}: {
  browser: Browser;
  sessionId: string;
  tabId?: string;
  disabled: boolean;
  /** Compact toolbar icons, with labelled entries when moved into overflow. */
  compact?: boolean;
  collapsed?: boolean;
  onChoose?: () => void;
  /** Local feedback shown below the preview toolbar. */
  onNotice: (text: string) => void;
  /** Hides only the overflow controls; a pending name prompt must remain mounted. */
  visible?: boolean;
}): ReactElement {
  const app = useApp();
  const blocked = useKeyReadiness("vault") !== null;
  const blockedRef = useRef(blocked);
  const [menuOpen, setMenuOpen] = useState(false);
  const [localChoice, setLocalChoice] = useState<VaultChoice | null>(null);
  const [busy, setBusy] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const more = useRef<HTMLButtonElement>(null);
  const choiceRef = useRef<VaultChoice | null>(null);
  const choosingRef = useRef(false);
  const mounted = useRef(false);
  if (!visible && menuOpen) setMenuOpen(false);
  const closeMenu = useCallback(() => {
    setMenuOpen(false);
  }, []);
  const menu = useBrowserPopup(RESERVATION, menuOpen && visible, more, closeMenu);

  const vaultBlockedAfterError = useCallback(async (): Promise<boolean> => {
    const keys = app.keys;
    if (keys !== null) {
      try {
        await keys.load();
      } catch {
        // Keep the last known readiness; the failed operation still stays local.
      }
    }
    return blockedRef.current || (keys !== null && vaultReadiness(keys.store.get()) !== null);
  }, [app.keys]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const pending = choiceRef.current;
      choiceRef.current = null;
      if (pending !== null)
        void browser.cancelSecret(sessionId, pending.handle, pending.tabId).catch(() => undefined);
    };
  }, [browser, sessionId]);

  const discard = useCallback(
    (choice: VaultChoice) => {
      void browser
        .cancelSecret(sessionId, choice.handle, choice.tabId)
        .catch(async (thrown: unknown) => {
          if (await vaultBlockedAfterError()) return;
          if (mounted.current)
            app.toasts.push({
              kind: "error",
              bellKind: "failure",
              text: toCallError(thrown).error.message,
            });
        });
    },
    [app, browser, sessionId, vaultBlockedAfterError],
  );
  const close = useCallback(() => {
    const pending = choiceRef.current;
    choiceRef.current = null;
    setLocalChoice(null);
    if (pending !== null) discard(pending);
  }, [discard]);

  useEffect(() => {
    const keys = app.keys;
    if (keys === null) return;
    return keys.store.subscribe(() => {
      const nowBlocked = vaultReadiness(keys.store.get()) !== null;
      blockedRef.current = nowBlocked;
      if (nowBlocked) {
        setMenuOpen(false);
        close();
      }
    });
  }, [app.keys, close]);

  const acceptChoice = useCallback(
    (next: VaultChoice) => {
      if (!mounted.current || blockedRef.current || choiceRef.current !== null) {
        discard(next);
        return;
      }
      choiceRef.current = next;
      setLocalChoice(next);
    },
    [discard],
  );

  const choose = useCallback(
    (mode: "selection" | "element" | "contextSelection") => {
      if (blockedRef.current || choosingRef.current || choiceRef.current !== null) return;
      onChoose?.();
      choosingRef.current = true;
      setMenuOpen(false);
      setChoosing(true);
      void closeBrowserOverlaysAndWait()
        .then(() => browser.chooseSecret(sessionId, mode, tabId))
        .then(acceptChoice, async (thrown: unknown) => {
          if (await vaultBlockedAfterError()) return;
          if (mounted.current) onNotice(toCallError(thrown).error.message);
        })
        .finally(() => {
          choosingRef.current = false;
          if (mounted.current) setChoosing(false);
        });
    },
    [acceptChoice, browser, sessionId, tabId, vaultBlockedAfterError, onChoose, onNotice],
  );

  useEffect(() => {
    const off = onContextSave((clickedSession) => {
      if (clickedSession === sessionId) choose("contextSelection");
    });
    return off;
  }, [choose, sessionId]);

  const save = (name: string) => {
    const activeChoice = choiceRef.current;
    if (blockedRef.current || activeChoice === null || busy) return;
    setBusy(true);
    void browser
      .saveSecret(sessionId, activeChoice.handle, name, activeChoice.tabId)
      .then(
        () => {
          if (choiceRef.current?.handle === activeChoice.handle) {
            choiceRef.current = null;
            setLocalChoice(null);
          }
          if (mounted.current && !blockedRef.current) onNotice(`Saved ${name} to the vault.`);
        },
        async () => {
          const blockedAfterError = await vaultBlockedAfterError();
          if (choiceRef.current?.handle === activeChoice.handle) close();
          if (mounted.current && !blockedAfterError)
            app.toasts.push({
              kind: "error",
              bellKind: "failure",
              text: "Save not completed. Select the value again to retry.",
            });
        },
      )
      .finally(() => {
        if (mounted.current) setBusy(false);
      });
  };

  return (
    <>
      <div className="preview-save-control">
        {compact && !collapsed ? (
          <IconButton
            icon="key-round"
            label="Save to vault"
            data-uc1-save="selection"
            disabled={blocked || disabled || choosing || localChoice !== null}
            onClick={() => {
              choose("selection");
            }}
          />
        ) : (
          <button
            type="button"
            className="preview-btn"
            data-uc1-save="selection"
            disabled={blocked || disabled || choosing || localChoice !== null}
            title="Save to vault"
            role={collapsed ? "menuitem" : undefined}
            onClick={() => {
              choose("selection");
            }}
          >
            Save to vault
          </button>
        )}
        <button
          type="button"
          className={compact ? "ibtn preview-save-more" : "preview-btn preview-save-more"}
          data-uc1-save="more"
          ref={more}
          aria-label="More ways to save"
          title="More ways to save"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          disabled={blocked || disabled || choosing || localChoice !== null}
          onClick={() => {
            setMenuOpen((open) => !open);
          }}
        >
          ▾
        </button>
        {menuOpen && !blocked && (
          <div
            ref={menu}
            className="preview-save-menu"
            role="menu"
            aria-label="Save to vault options"
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                setMenuOpen(false);
                more.current?.focus();
              }
            }}
          >
            <button
              type="button"
              role="menuitem"
              data-uc1-save="element"
              onClick={() => {
                choose("element");
              }}
            >
              Click an element
            </button>
          </div>
        )}
      </div>
      {localChoice !== null &&
        !blocked &&
        createPortal(
          <SaveNameDialog
            key={localChoice.handle}
            choice={localChoice}
            busy={busy}
            onClose={close}
            onSave={save}
          />,
          document.body,
        )}
    </>
  );
}
