/**
 * A single Vault entry, inline project permission and expandable actions/grants. Depends on metadata
 * listing and actions. Called by ProjectSecretsSettings; never reads or reveals stored values.
 */
import { useId, useState } from "react";
import { Button, RowDisclosure } from "../../components/Controls";
import { Selector } from "../../components/Selector";
import type { ProjectSecretView, LoginView } from "../../protocol/core";
import { term, vocabulary } from "../../vocabulary";
import { McpTag, isMcp } from "./McpLabel";
import { VaultItemActions } from "./VaultItemActions";
import { originHost, type EntryPermission, type VaultAct, type VaultListing } from "./vaultEntries";

/** Values and labels shared by add and inline permission controls. */
export const PERMISSIONS: ReadonlyArray<readonly [EntryPermission, string]> = Object.entries(
  vocabulary.vaultPermissions,
).map(([id, value]) => [id as EntryPermission, value.name]);

/** Permission choices share the same menu and type scale as every other selector. */
export const PERMISSION_OPTIONS = PERMISSIONS.map(([value, label]) => ({
  value,
  label,
  tooltip: term("vaultPermissions", value).description,
  tone:
    value === "allowed"
      ? ("allowed" as const)
      : value === "never"
        ? ("never" as const)
        : ("neutral" as const),
}));

/** Metadata discriminated by entry kind. */
export type VaultEntry =
  { kind: "secret"; item: ProjectSecretView } | { kind: "login"; item: LoginView };

/** Main address: the bound MCP endpoint takes precedence over its capture site. */
export function entryAddress(entry: VaultEntry, list: VaultListing): string {
  if (entry.kind === "login") return entry.item.origin;
  return (
    list.serverGrants.find((g) => g.secretId === entry.item.id)?.address ??
    entry.item.sourceOrigin ??
    ""
  );
}

/** Focusable one-line disclosure and chip, followed by actions only when expanded. */
export function VaultEntryRow(props: {
  entry: VaultEntry;
  list: VaultListing;
  project: string | null;
  busy: boolean;
  expanded: boolean;
  onExpandedChange: () => void;
  act: VaultAct;
  replacement?: number | null;
}) {
  const { entry, list, busy, act } = props;
  const { item } = entry;
  const id = useId();
  const [showProjects, setShowProjects] = useState(false);
  const project = props.project;
  const address = entryAddress(entry, list);
  const defaultPermission = item.defaultPermission;
  const projectPermission = item.projectPermission;
  const projectPermissions = item.projectPermissions;
  const inherited = `Default (${term("vaultPermissions", defaultPermission).name})`;
  const grants =
    entry.kind === "secret"
      ? list.grants
          .filter((g) => g.secretIds.includes(item.id))
          .map((g) => ({ id: g.id, label: g.command, detail: g.project }))
      : list.fillGrants
          .filter((g) => g.loginId === item.id)
          .map((g) => ({ id: g.id, label: g.origin, detail: "Sign-in" }));
  const servers =
    entry.kind === "secret" ? list.serverGrants.filter((g) => g.secretId === item.id) : [];
  return (
    <div
      data-item-id={props.entry.item.id}
      data-setting={`vault-login-replace-${encodeURIComponent(item.id)}`}
      className="vault-entry"
      {...(entry.kind === "secret"
        ? { "data-secret-row": item.id }
        : { "data-login-row": item.id })}
    >
      <div className="check secrets-row vault-entry-line">
        <RowDisclosure
          data-testid="secrets.vault-entry-row-rowdisclosure"
          type="button"
          aria-label={`Show details for ${item.name}`}
          aria-expanded={props.expanded}
          aria-controls={props.expanded ? id : undefined}
          onClick={props.onExpandedChange}
        >
          <span className="secrets-name ui-body" title={item.name}>
            {item.name}
          </span>
          <span className="secrets-detail ui-caption vault-entry-address" title={address}>
            {address ? originHost(address) : "—"}
          </span>
          {entry.kind === "login" && (
            <span className="secrets-detail ui-caption" title={entry.item.username}>
              {entry.item.username}
            </span>
          )}
        </RowDisclosure>
        <span className="ui-caption">Default permission</span>
        <Selector
          testId="secrets.vault-entry-row-act"
          compact
          label={`Default permission for ${item.name}`}
          value={defaultPermission}
          disabled={busy || !list.unlocked}
          options={PERMISSION_OPTIONS}
          onChange={(permission) => {
            act(
              { op: "setDefaultPermission", id: item.id, permission },
              "Default permission saved.",
            );
          }}
        />
        <Button
          data-testid="secrets.vault-entry-row-set-show-projects"
          small
          type="button"
          aria-expanded={showProjects}
          aria-label={`Project permissions for ${item.name}`}
          onClick={() => {
            setShowProjects(!showProjects);
          }}
        >
          Project permissions
        </Button>
      </div>
      {showProjects && (
        <div
          className="vault-project-permissions"
          aria-label={`Project permissions for ${item.name}`}
        >
          {project !== null && (
            <div className="vault-project-permission-row">
              <span title={project}>{project}</span>
              <Selector
                testId="secrets.vault-entry-row-act-2"
                compact
                label={`Project permission for ${item.name}`}
                value={projectPermission ?? "default"}
                disabled={busy || !list.unlocked}
                options={[{ value: "default", label: inherited }, ...PERMISSION_OPTIONS]}
                onChange={(permission) => {
                  if (permission === "default")
                    act(
                      { op: "resetEntryPermission", id: item.id, project },
                      "Project permission reset.",
                    );
                  else
                    act(
                      {
                        op: "setEntryPermission",
                        id: item.id,
                        project,
                        permission,
                      },
                      "Project permission saved.",
                    );
                }}
              />
            </div>
          )}
          {projectPermissions
            .filter((p) => p.project !== project)
            .map((p) => (
              <div
                data-item-id={p.project}
                className="vault-project-permission-row"
                key={p.project}
              >
                <span title={p.project}>{p.project}</span>
                <span>{term("vaultPermissions", p.permission).name}</span>
                <Button
                  data-testid="secrets.vault-entry-row-act-3"
                  small
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    act(
                      { op: "resetEntryPermission", id: item.id, project: p.project },
                      "Project permission reset.",
                    );
                  }}
                >
                  Reset to default
                </Button>
              </div>
            ))}
        </div>
      )}
      {props.expanded && (
        <div id={id} className="vault-entry-details">
          <VaultItemActions
            key={props.replacement ?? "actions"}
            {...(entry.kind === "login" && entry.item.own && props.replacement != null
              ? { replaceFromFinding: entry.item.origin }
              : {})}
            inline
            kind={entry.kind}
            name={item.name}
            own={item.own}
            busy={busy}
            onRename={(name) => {
              act(
                { op: entry.kind === "login" ? "renameLogin" : "rename", id: item.id, name },
                "Renamed.",
              );
            }}
            onReplace={(value) => {
              act(
                entry.kind === "login"
                  ? { op: "replaceLoginPassword", id: item.id, password: value }
                  : { op: "replace", id: item.id, value },
                "Replaced.",
              );
            }}
            onGenerate={(options) => {
              act(
                {
                  op: entry.kind === "login" ? "regenerateLoginPassword" : "regenerate",
                  id: item.id,
                  options,
                },
                "Generated a new value. Copied. Clears in 30 s.",
              );
            }}
            onCopy={() => {
              act(
                { op: entry.kind === "login" ? "copyLoginPassword" : "copy", id: item.id },
                "Copied. Clears in 30 s.",
              );
            }}
            {...(entry.kind === "login"
              ? {
                  onCopyUsername: () => {
                    act({ op: "copyLoginUsername", id: item.id }, "Copied. Clears in 30 s.");
                  },
                }
              : {})}
            onRemove={() => {
              act(
                { op: entry.kind === "login" ? "removeLogin" : "remove", id: item.id },
                "Removed.",
              );
            }}
            projectPermissions={projectPermissions.map((p) => p.project)}
          />
          {entry.kind === "secret" && isMcp(entry.item) && (
            <p className="key-field-hint">
              <McpTag /> Key for an MCP server.
            </p>
          )}
          {!item.own && <p className="key-field-hint">Managed in another vault app.</p>}
          {grants.length > 0 && (
            <h4>{entry.kind === "login" ? "Granted sign-ins" : "Granted commands"}</h4>
          )}
          {grants.map((g) => (
            <div data-item-id={g.id} className="check secrets-row" key={g.id} data-grant={g.id}>
              <code className="secrets-name" title={`${g.label} · ${g.detail}`}>
                {g.label}
              </code>
              <Button
                data-testid="secrets.vault-entry-row-act-4"
                small
                type="button"
                disabled={busy}
                onClick={() => {
                  act({ op: "revoke", grantId: g.id }, "Revoked.");
                }}
              >
                Revoke
              </Button>
            </div>
          ))}
          {servers.length > 0 && <h4>MCP servers</h4>}
          {servers.map((g) => (
            <div
              data-item-id={g.id}
              className="check secrets-row"
              key={g.id}
              data-server-grant={g.id}
            >
              <span className="secrets-name" title={g.address}>
                {g.server}
              </span>
              <span className="secrets-detail d">
                {g.grant === "none"
                  ? "Not allowed"
                  : g.grant === "always"
                    ? "Always"
                    : "Until close"}
              </span>
              <Button
                data-testid="secrets.vault-entry-row-act-5"
                small
                type="button"
                disabled={busy || g.grant === "none"}
                onClick={() => {
                  act({ op: "revoke", grantId: g.id }, "Revoked.");
                }}
              >
                Revoke
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
