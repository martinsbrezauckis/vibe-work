/**
 * Shared password generation controls for add and replacement forms. Only the options cross the
 * bridge; the generated value stays in the core. Depends on React and the shared Settings field/control
 * styles. Called by the secrets and logins settings forms and the replacement editor.
 */
import { Button } from "../../components/Controls";
import { type ReactNode, useId, useState, useSyncExternalStore } from "react";

/** The options sent to the vault generator. */
export interface GenerateOptions {
  length: number;
  uppercase: boolean;
  digits: boolean;
  symbols: boolean;
}

// The disclosure preference lasts only as long as this app window's JavaScript session.
const expandedForms = new Map<string, boolean>();
const disclosureListeners = new Set<() => void>();

function subscribeDisclosure(listener: () => void) {
  disclosureListeners.add(listener);
  return () => disclosureListeners.delete(listener);
}

function setDisclosureExpanded(scope: string, expanded: boolean) {
  expandedForms.set(scope, expanded);
  disclosureListeners.forEach((listener) => {
    listener();
  });
}

/** Owns one form's generator choices and validates their bounds before enabling Generate. */
export function useGeneratorOptions() {
  const [options, setOptions] = useState<GenerateOptions>({
    length: 24,
    uppercase: true,
    digits: true,
    symbols: true,
  });
  const valid = Number.isInteger(options.length) && options.length >= 8 && options.length <= 128;
  return { options, setOptions, valid };
}

/** Compact controls shared by the two Add forms and the row replacement editor. */
export function GeneratorOptions(props: {
  options: GenerateOptions;
  setOptions: (options: GenerateOptions) => void;
  /** The form's single Generate action, placed beside its Options disclosure. */
  children?: ReactNode;
  /** Remembers this form's disclosure independently for the current app session. */
  scope?: string;
}) {
  const { options, setOptions } = props;
  const lengthId = useId();
  const panelId = useId();
  const scope = props.scope ?? "default";
  const expanded = useSyncExternalStore(
    subscribeDisclosure,
    () => expandedForms.get(scope) ?? false,
  );
  return (
    <div className="generator-options key-field" data-generator-options="true">
      <div className="key-actions">
        {props.children}
        <Button
          data-testid="secrets.generator-options-set-disclosure-expanded"
          type="button"

          aria-expanded={expanded}
          aria-controls={expanded ? panelId : undefined}
          onClick={() => {
            setDisclosureExpanded(scope, !expanded);
          }}
        >
          Options
        </Button>
      </div>
      {expanded && (
        <div id={panelId} className="key-offer" role="group" aria-label="Generation options">
          <div className="key-field">
            <label htmlFor={lengthId}>Length</label>
            <div className="key-field-row">
              <input
                data-testid="secrets.generator-options-set-options"
                id={lengthId}
                type="number"
                min={8}
                max={128}
                step={1}
                value={Number.isNaN(options.length) ? "" : options.length}
                onChange={(event) => {
                  setOptions({ ...options, length: event.target.valueAsNumber });
                }}
              />
            </div>
          </div>
          <div className="key-actions">
            {(["uppercase", "digits", "symbols"] as const).map((key) => (
              <label data-item-id={key} className="key-check" key={key}>
                <input
                  data-testid="secrets.generator-options-set-options-2"
                  type="checkbox"
                  checked={options[key]}
                  onChange={(event) => {
                    setOptions({ ...options, [key]: event.target.checked });
                  }}
                />
                {key === "uppercase" ? "Uppercase" : key === "digits" ? "Digits" : "Symbols"}
              </label>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
