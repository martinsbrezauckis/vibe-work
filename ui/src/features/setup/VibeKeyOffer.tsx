/**
 * "Vibe already has a key on this computer. Use it?": the offer to take the key Vibe itself saved
 * (its `.env` file or its keychain entry) into the vault. Shown on the first-run create step and on
 * the key step. On macOS the text says the copy can be removed from the keychain afterwards; the copy
 * is only removed when the user presses the button, never silently.
 * Depends on: protocol/core types. Called by: CreateStep, KeyStep.
 */
import { Button } from "../../components/Controls";
import type { VibeKeySource, VibeKeyView } from "../../protocol/core";

/** Plain name of where Vibe keeps it. */
export function placeOf(found: VibeKeyView): string {
  return found.source === "dotEnv" ? "its settings file" : "the system keychain";
}

/** The explanation shown above the choice. */
export function VibeKeyNote({ found }: { found: VibeKeyView }) {
  return (
    <>
      <p>Vibe already has a key on this computer (in {placeOf(found)}). Use it?</p>
      {found.macKeychain && (
        <p className="key-field-hint">
          After you move it into the vault you can remove Vibe&apos;s copy from the keychain.
        </p>
      )}
    </>
  );
}

/** The one-click button of the key step. */
export function UseVibeKey(props: {
  found: VibeKeyView;
  busy: boolean;
  onUse: (source: VibeKeySource) => void;
}) {
  return (
    <div data-item-id={props.found.source} className="key-offer">
      <VibeKeyNote found={props.found} />
      <Button
        data-testid="setup.vibe-key-offer-on-use"
        type="button"
        appearance="dialog"
        disabled={props.busy}
        onClick={() => {
          props.onUse(props.found.source);
        }}
      >
        Use Vibe&apos;s key
      </Button>
    </div>
  );
}
