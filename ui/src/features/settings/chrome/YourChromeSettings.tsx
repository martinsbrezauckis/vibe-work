/**
 * Settings › Browser › Your Chrome: the one place where the user decides how
 * much they trust Vibe in their own Chrome.
 * - The status row: not set up (Set up…), waiting for the extension, or set up (profiles).
 * - "What Vibe may do in your Chrome": three level cards (Basic browser control is the default), each with one contract
 *   line. Anything above the level, and bookmarks and history at every level, Vibe asks about with the card.
 * - Raising the level passes the core's native confirmation; Cancel keeps the old level ("Not changed."); lowering
 *   saves at once ("Saved. Applies from Vibe's next action in Chrome.").
 * - Closing tabs you opened stays its own setting under Delegation permissions (it covers both browsers).
 * Depends on: Controls, Badge, useChromeControl, chromeRows, ChromeDialogs. Called by: BrowserSettings.
 * Invariants: renders only what the core confirmed; arrows move between level cards without choosing (choosing a
 * higher card opens a native confirmation, so passing one on the way must not).
 */
import { type KeyboardEvent, type ReactElement, useRef, useState } from "react";

import { Badge } from "../../../components/Badge";
import { Button, SettingsRow } from "../../../components/Controls";
import type { ChromeLevel } from "../../../protocol/core";
import { TEXT, term } from "../../../vocabulary";
import { ChromeRemoveDialog, ChromeSetupDialog } from "./ChromeDialogs";
import { LEVELS, chromeAvailable, chromeName, contract, setupLine } from "./chromeRows";
import { useChromeControl } from "./useChromeControl";

/** The three level cards, one radio group with a roving focus. */
function LevelCards({
  chosen,
  disabled,
  lowered,
  onChoose,
}: {
  chosen: ChromeLevel | null;
  disabled: boolean;
  lowered: boolean;
  onChoose: (level: ChromeLevel) => void;
}): ReactElement {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const [focus, setFocus] = useState(Math.max(0, chosen === null ? 0 : LEVELS.indexOf(chosen)));
  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    const step =
      event.key === "ArrowRight" || event.key === "ArrowDown"
        ? 1
        : event.key === "ArrowLeft" || event.key === "ArrowUp"
          ? -1
          : 0;
    if (step === 0) return;
    event.preventDefault();
    const next = (focus + step + LEVELS.length) % LEVELS.length;
    setFocus(next);
    refs.current[next]?.focus();
  };
  return (
    <div
      className="vault-choices chrome-levels"
      role="radiogroup"
      data-testid="settings.chrome-levels"
      aria-labelledby="chrome-level-title"
      aria-busy={disabled}
      data-setting="chrome-control-level"
      onKeyDown={move}
    >
      {LEVELS.map((level, index) => (
        <button
          key={level}
          ref={(el) => {
            refs.current[index] = el;
          }}
          type="button"
          role="radio"
          aria-checked={chosen === level}
          aria-describedby={`chrome-level-${level}`}
          tabIndex={index === focus ? 0 : -1}
          className="vault-choice"
          data-testid="settings.chrome-level"
          data-item-id={level}
          disabled={disabled}
          onClick={() => {
            if (chosen !== level || lowered) onChoose(level);
          }}
        >
          <strong>
            {chromeName(level)}
            {level === "browse" ? <Badge>Default</Badge> : null}
            {chosen === level ? <Badge tone="ok">Chosen</Badge> : null}
          </strong>
          <span id={`chrome-level-${level}`}>{contract(level)}</span>
        </button>
      ))}
    </div>
  );
}

/** The section. */
export function YourChromeSettings(): ReactElement | null {
  const chrome = useChromeControl();
  const [setupOpen, setSetupOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const view = chrome.view;
  const setUp = view !== null && view.setup.state !== "notSetUp";
  const busy = chrome.busy !== null || view === null;
  if (!chromeAvailable(view)) return null;
  return (
    <section aria-labelledby="chrome-title" data-setting="chrome-control">
      <h3 id="chrome-title">Your Chrome</h3>
      {view === null && chrome.readError === null ? (
        <p className="key-line" role="status">
          {TEXT.chromeReading}
        </p>
      ) : null}
      <SettingsRow data-setting="chrome-control-status">
        <span>
          Chrome Control
          <span className="d" role="status">
            {chrome.readError !== null && view === null
              ? `Could not read Chrome Control's state: ${chrome.readError}`
              : setupLine(view)}
          </span>
        </span>
        {setUp ? (
          <Button
            data-testid="settings.chrome-setup-open"
            shape="outlined"
            onClick={() => {
              setSetupOpen(true);
            }}
          >
            {view.setup.pairing === "Paired" ? "Details" : "Finish setting up…"}
          </Button>
        ) : (
          <Button
            data-testid="settings.chrome-setup-open"
            shape="outlined"
            disabled={view === null || !view.setup.package}
            onClick={() => {
              setSetupOpen(true);
            }}
          >
            Set up…
          </Button>
        )}
      </SettingsRow>
      {view?.lowered ? (
        <p className="ui-help">
          Chrome lowered some permissions. Choose a level again to change them.
        </p>
      ) : null}
      <h4 id="chrome-level-title">{TEXT.chromeLevelTitle}</h4>
      <p className="ui-help">{TEXT.chromeLevelHelp}</p>
      <LevelCards
        chosen={view?.level ?? null}
        disabled={busy}
        lowered={view?.lowered ?? false}
        onChoose={(level) => {
          void chrome.run({ op: "setLevel", level }, "setLevel");
        }}
      />
      <p className="ui-help" data-chrome-close-note="">
        Closing tabs you opened is its own setting, “
        {term("browserPermissions", "closeUserTabs").name}”, under {TEXT.browserDelegationTitle}{" "}
        below. It covers Vibe Work&apos;s browser and your Chrome.
      </p>
      <p
        className={chrome.status?.alert === true ? "key-problem" : "key-line"}
        role={chrome.status?.alert === true ? "alert" : "status"}
        aria-live="polite"
        data-chrome-status=""
      >
        {chrome.status?.text ?? ""}
      </p>
      {setUp ? (
        <SettingsRow data-setting="chrome-control-remove">
          <span>
            Remove Chrome Control
            <span className="d">Stops using your Chrome and deletes its saved level.</span>
          </span>
          <Button
            data-testid="settings.chrome-remove-open"
            shape="outlined"
            variant="danger"
            onClick={() => {
              setRemoveOpen(true);
            }}
          >
            Remove…
          </Button>
        </SettingsRow>
      ) : null}
      <ChromeSetupDialog
        open={setupOpen}
        chrome={chrome}
        onClose={() => {
          setSetupOpen(false);
        }}
      />
      <ChromeRemoveDialog
        open={removeOpen}
        chrome={chrome}
        onClose={() => {
          setRemoveOpen(false);
        }}
      />
    </section>
  );
}
