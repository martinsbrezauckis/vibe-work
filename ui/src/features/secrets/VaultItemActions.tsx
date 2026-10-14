/**
 * Compact actions for a listed vault item. The row contains metadata only; a replacement value
 * exists solely in this temporary controlled field and is cleared as soon as it is submitted.
 * Generation sends options only and requires confirmation before replacing an existing value.
 * Depends on: SecretField, GeneratorOptions. Called by: VaultEntryRow.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { Button } from "../../components/Controls";

import { SecretField } from "../setup/fields";
import { GeneratorOptions, useGeneratorOptions, type GenerateOptions } from "./GeneratorOptions";

type Mode = "closed" | "menu" | "rename" | "replace" | "confirm-remove";

/** Row-end actions. The caller starts each request synchronously inside the supplied callback. */
export function VaultItemActions(props: {
  kind: "secret" | "login";
  name: string;
  own: boolean;
  busy: boolean;
  /** Show actions directly inside an expanded entry instead of a row-end menu. */
  inline?: boolean;
  onRename: (name: string) => void;
  onReplace: (value: string) => void;
  onGenerate: (options: GenerateOptions) => void;
  onCopy: () => void;
  onCopyUsername?: () => void;
  onRemove: () => void;
  projectPermissions?: string[];
}) {
  const [mode, setMode] = useState<Mode>(props.inline === true ? "menu" : "closed");
  const [name, setName] = useState(props.name);
  const [value, setValue] = useState("");
  const [generating, setGenerating] = useState(false);
  const generator = useGeneratorOptions();
  const [above, setAbove] = useState(false);
  const [editorPosition, setEditorPosition] = useState<{ top: number; left: number } | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLElement | null>(null);
  const copyUsername = props.onCopyUsername;

  const close = useCallback(
    (returnFocus = true) => {
      setValue("");
      setGenerating(false);
      setMode(props.inline === true ? "menu" : "closed");
      if (returnFocus) trigger.current?.focus();
    },
    [props.inline],
  );

  useEffect(() => {
    if (mode === "closed" || props.inline === true) return;
    const onOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !root.current?.contains(event.target)) close(false);
    };
    document.addEventListener("pointerdown", onOutside);
    return () => {
      document.removeEventListener("pointerdown", onOutside);
    };
  }, [mode, close, props.inline]);

  useLayoutEffect(() => {
    if (props.inline === true || (mode !== "menu" && mode !== "replace")) return;
    const measure = () => {
      const row = root.current?.closest(".secrets-row");
      if (row === null || row === undefined || panel.current === null) return;
      const bounds = row.getBoundingClientRect();
      const popup = panel.current;
      const height = popup.getBoundingClientRect().height;
      if (mode === "menu") {
        const below = window.innerHeight - bounds.bottom;
        setAbove(below < height + 8 && bounds.top > below);
        return;
      }
      // A taller generated-password editor cannot remain row-absolute inside the scrolling
      // settings body: it is clipped at the viewport edge. Keep it in viewport coordinates.
      const footer = document.querySelector(".statusbar")?.getBoundingClientRect();
      const bottom = footer?.top ?? window.innerHeight;
      const available = bottom - 8;
      const width = popup.getBoundingClientRect().width || Math.min(290, window.innerWidth - 32);
      const top = Math.max(8, Math.min(bounds.bottom + 2, available - height));
      const rowRight = Number.isFinite(bounds.right) ? bounds.right : window.innerWidth - 16;
      const left = Math.max(8, Math.min(rowRight - width, window.innerWidth - width - 8));
      setEditorPosition((current) =>
        current?.top === top && current.left === left ? current : { top, left },
      );
    };
    measure();
    const resizeObserver =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    if (panel.current !== null) resizeObserver?.observe(panel.current);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      resizeObserver?.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [mode, generating, props.inline]);

  const choose = (action: () => void) => {
    action();
    close();
  };

  return (
    <div
      ref={root}
      className={`vault-actions${props.inline === true ? " vault-actions-inline" : ""}`}
      onKeyDown={(event) => {
        if (event.key === "Escape" && mode !== "closed") {
          event.preventDefault();
          event.stopPropagation();
          close();
        } else if (event.key === "Enter" && (mode === "rename" || mode === "replace")) {
          event.stopPropagation();
        }
      }}
    >
      {props.inline !== true && (
        <button
          ref={trigger}
          type="button"
          className="tb-btn vault-actions-trigger"
          aria-label={`Actions for ${props.name}`}
          aria-expanded={mode !== "closed"}
          disabled={props.busy}
          onClick={() => {
            if (mode === "closed") setMode("menu");
            else close();
          }}
        >
          Actions
        </button>
      )}
      {mode === "menu" && (
        <div
          ref={(node) => {
            panel.current = node;
          }}
          className="vault-actions-menu"
          data-placement={above ? "above" : "below"}
          role="group"
          aria-label={`Actions for ${props.name}`}
        >
          {props.own && (
            <>
              <Button
                small
                type="button"

                onClick={() => {
                  setName(props.name);
                  setMode("rename");
                }}
              >
                Rename
              </Button>
              <Button
                small
                type="button"

                onClick={() => {
                  setMode("replace");
                }}
              >
                Replace {props.kind === "login" ? "password" : "value"}
              </Button>
            </>
          )}
          {props.kind === "login" && copyUsername !== undefined && (
            <Button
              small
              type="button"

              onClick={() => {
                choose(copyUsername);
              }}
            >
              Copy username
            </Button>
          )}
          <Button
            small
            type="button"

            onClick={() => {
              choose(props.onCopy);
            }}
          >
            Copy {props.kind === "login" ? "password" : "value"}
          </Button>
          {props.own && (
            <Button
              small
              disabled={props.busy}
              onClick={() => {
                setValue("");
                setMode("replace");
                setGenerating(true);
              }}
            >
              Generate replacement {props.kind === "login" ? "password" : "value"}
            </Button>
          )}
          {props.own && (
            <Button
              small
              type="button"

              onClick={() => {
                setMode("confirm-remove");
              }}
            >
              Remove
            </Button>
          )}
        </div>
      )}
      {mode === "confirm-remove" && (
        <div
          className="vault-actions-editor vault-remove-confirm"
          role="group"
          aria-label={`Confirm removal of ${props.name}`}
        >
          <p>Delete {props.name}? This cannot be undone.</p>
          <p>Project permissions: {props.projectPermissions?.join(", ") || "None"}</p>
          <Button
            small
            type="button"
            variant="danger"
            disabled={props.busy}
            onClick={() => {
              choose(props.onRemove);
            }}
          >
            Delete {props.name}
          </Button>
          <Button
            small
            type="button"
            autoFocus
            onClick={() => {
              close();
            }}
          >
            Cancel
          </Button>
        </div>
      )}
      {mode === "rename" && (
        <form
          className="vault-actions-editor key-field-row is-rename"
          onSubmit={(event) => {
            event.preventDefault();
            if (name.trim() !== "" && name.trim() !== props.name)
              choose(() => {
                props.onRename(name.trim());
              });
          }}
        >
          <input
            aria-label="Name"
            autoFocus
            value={name}
            maxLength={80}
            autoComplete="off"
            onChange={(event) => {
              setName(event.target.value);
            }}
          />
          <Button
            small
            type="submit"
            variant="primary"
            disabled={name.trim() === "" || name.trim() === props.name}
          >
            Save
          </Button>
          <Button
            small
            type="button"

            onClick={() => {
              close();
            }}
          >
            Cancel
          </Button>
        </form>
      )}
      {mode === "replace" && (
        <form
          ref={(node) => {
            panel.current = node;
          }}
          className="vault-actions-editor key-actions"
          data-placement="viewport"
          style={{ top: editorPosition?.top, left: editorPosition?.left }}
          onSubmit={(event) => {
            event.preventDefault();
            if (generating) return;
            if (value === "" || (props.kind === "login" && value.length < 8)) return;
            const replacement = value;
            props.onReplace(replacement);
            close();
          }}
        >
          {!generating && (
            <SecretField
              label={props.kind === "login" ? "New password" : "New value"}
              value={value}
              onChange={setValue}
              autoComplete="off"
              autoFocus
            />
          )}
          {generating && (
            <GeneratorOptions
              scope={`replace-${props.kind}`}
              options={generator.options}
              setOptions={generator.setOptions}
            >
              <div
                className="generator-replace key-actions"
                role="group"
                aria-label="Generate replacement options"
              >
                <Button
                  small
                  type="button"
                  variant="primary"
                  disabled={props.busy || !generator.valid}
                  onClick={() => {
                    // Start the user-only command in this click; do not defer it behind a promise.
                    props.onGenerate(generator.options);
                    close();
                  }}
                >
                  Generate replacement {props.kind === "login" ? "password" : "value"}
                </Button>
                <Button
                  small
                  type="button"

                  onClick={() => {
                    close();
                  }}
                >
                  Cancel
                </Button>
              </div>
            </GeneratorOptions>
          )}
          {!generating && (
            <>
              <Button
                small
                type="submit"
                variant="primary"
                disabled={value === "" || (props.kind === "login" && value.length < 8)}
              >
                Save
              </Button>
              <Button
                small
                type="button"

                onClick={() => {
                  close();
                }}
              >
                Cancel
              </Button>
            </>
          )}
        </form>
      )}
    </div>
  );
}
