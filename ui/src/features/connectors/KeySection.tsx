/**
 * "This server needs an API key" in the Add MCP server dialog: pick a vault secret (the project secrets
 * of Settings > Project secrets) or add a new one, and the header that carries it. The key stays in the vault:
 * Vibe's config gets only an address on this computer, and Vibe Work adds the key to each request.
 * Depends on: AppContext (core), state/secrets (list), setup/useKeys (vault status), setup/fields (SecretField), model/keyDraft. Called by:
 * AddServerDialog. A missing or locked vault is explained here, never by replacing the Tools page.
 * Invariants: the value field is a `SecretField` (`data-secret`); the list shows names only (the core
 * never sends a value); nothing is stored here: the core stores a new secret only after the user's native dialog.
 */
import { type ReactElement, useEffect, useRef, useState } from "react";

import { Selector } from "../../components/Selector";
import type { KeyStatusView, ProjectSecretView } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { problem, secrets } from "../../state/secrets";
import { isMcp, mcpFirst } from "../secrets/McpLabel";
import { SecretField } from "../setup/fields";
import { useKeyUi } from "../setup/useKeys";
import { useApp } from "../shell/AppContext";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { type KeyDraft, headerProblem } from "./model/keyDraft";

/**
 * The secrets in picker order: keys labelled for MCP servers first, the others still selectable after
 * them. Groups get a heading only when both kinds exist. Order is a suggestion; every key can be picked.
 */
function pickerGroups(list: readonly ProjectSecretView[]): {
  label: string | null;
  items: ProjectSecretView[];
}[] {
  const ordered = mcpFirst(list);
  const mcp = ordered.filter(isMcp);
  const rest = ordered.filter((s) => !isMcp(s));
  if (mcp.length === 0 || rest.length === 0) return [{ label: null, items: ordered }];
  return [
    { label: "MCP keys", items: mcp },
    { label: "Other keys", items: rest },
  ];
}

/** The section; `draft` is owned by the dialog. */
export function KeySection(props: {
  draft: KeyDraft;
  onChange: (draft: KeyDraft) => void;
  onAvailableChange?: (available: boolean, status: KeyStatusView | null) => void;
}): ReactElement {
  const { core, keys } = useApp();
  const vaultStatus = useKeyUi((s) => s.status);
  const vaultBlocked = useKeyReadiness("vault") !== null;
  const { draft, onChange, onAvailableChange } = props;
  const [savedList, setList] = useState<readonly ProjectSecretView[] | null>(null);
  const list = vaultBlocked ? null : savedList;
  const [error, setError] = useState<string | null>(null);
  // The latest draft for the list's arrival (the effect below runs once per core, not per keystroke).
  const latest = useRef({ draft, onChange, onAvailableChange });
  useEffect(() => {
    latest.current = { draft, onChange, onAvailableChange };
  });
  const set = (p: Partial<KeyDraft>) => {
    onChange({ ...draft, ...p });
  };

  useEffect(() => {
    if (core === null || vaultBlocked) return;
    let live = true;
    secrets(core, { op: "list" })
      .then((r) => {
        if (!live || r.kind !== "list") return;
        if (!r.unlocked) {
          setList(null);
          setError(null);
          latest.current.onAvailableChange?.(false, vaultStatus);
          void keys?.load();
          return;
        }
        setList(r.secrets);
        setError(null);
        latest.current.onAvailableChange?.(true, vaultStatus);
        // With secrets to pick from, start on "A secret in your vault" unless the user already typed a new one.
        const { draft: now, onChange: change } = latest.current;
        if (r.secrets.length > 0 && now.mode === "new" && now.newName === "" && now.newValue === "")
          change({ ...now, mode: "existing" });
      })
      .catch((thrown: unknown) => {
        if (!live) return;
        if (toCallError(thrown).error.serverCode === "passwordRequired") {
          setList(null);
          setError(null);
          latest.current.onAvailableChange?.(false, vaultStatus);
          void keys?.load();
          return;
        }
        setError(problem(thrown));
      });
    return () => {
      live = false;
    };
  }, [core, keys, vaultBlocked, vaultStatus]);

  const headerCheck = draft.headerMode === "custom" ? headerProblem(draft.header) : null;
  return (
    <div className="key-server" data-key-section="true">
      {vaultBlocked ? (
        <KeyNotice
          requirement="vault"
          message="MCP server keys use your vault to keep their values encrypted."
        />
      ) : null}
      {vaultBlocked || error === null ? null : <p className="cbad">{error}</p>}
      <fieldset className="cchoice">
        <legend>The key</legend>
        <Selector
          label="Key source"
          value={draft.mode}
          options={[
            {
              value: "existing",
              label: "A secret in your vault",
              disabled: list === null || list.length === 0,
            },
            { value: "new", label: "A new secret" },
          ]}
          onChange={(mode) => {
            if (mode === "new" || (list !== null && list.length > 0)) set({ mode });
          }}
        />
        {draft.mode === "existing" ? (
          <Selector
            label="Secret"
            value={draft.secretId}
            onChange={(value) => {
              set({ secretId: value });
            }}
            options={[
              { value: "", label: "Pick a secret" },
              ...pickerGroups(list ?? []).flatMap((group) =>
                group.items.map((secret) => ({
                  value: secret.id,
                  label: secret.name,
                  ...(group.label === null ? {} : { description: group.label }),
                })),
              ),
            ]}
          />
        ) : null}
        {draft.mode === "new" ? (
          <>
            <label className="cfield">
              <span>Its name</span>
              <input
                value={draft.newName}
                placeholder="GitHub token"
                onChange={(e) => {
                  set({ newName: e.target.value });
                }}
              />
            </label>
            <SecretField
              label="The key or token"
              value={draft.newValue}
              onChange={(v) => {
                set({ newValue: v });
              }}
              hint="Stored in your vault after you confirm. Vibe and the agent never see it."
            />
          </>
        ) : null}
      </fieldset>
      <fieldset className="cchoice">
        <legend>How the server gets it</legend>
        <Selector
          label="How the server gets it"
          value={draft.headerMode}
          options={[
            { value: "bearer", label: "Authorization: Bearer … (most servers)" },
            { value: "custom", label: "Another header" },
          ]}
          onChange={(headerMode) => {
            set({ headerMode });
          }}
        />
        {draft.headerMode === "custom" ? (
          <label className="cfield">
            <span>Header name</span>
            <input
              value={draft.header}
              placeholder="X-Api-Key"
              spellCheck={false}
              aria-invalid={headerCheck !== null}
              onChange={(e) => {
                set({ header: e.target.value });
              }}
            />
            {headerCheck === null ? null : <span className="cbad">{headerCheck}</span>}
          </label>
        ) : null}
      </fieldset>
    </div>
  );
}
