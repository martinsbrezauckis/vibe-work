/**
 * One compact add form for secrets, logins and MCP keys. Depends on typed requests, SecretField and
 * Settings controls. Called by the Vault page; user input clears before dispatch, values never return.
 */
import { Button } from "../../components/Controls";
import { useState } from "react";
import { Selector } from "../../components/Selector";
import type { SecretsRequest } from "../../protocol/core";
import { term } from "../../vocabulary";
import { SecretField } from "../setup/fields";
import { GeneratorOptions, useGeneratorOptions } from "./GeneratorOptions";
import { PERMISSION_OPTIONS } from "./VaultEntryRow";
import type { EntryPermission, VaultAct, VaultListing } from "./vaultEntries";

/** Expands from Add entry and closes only after successful storage. */
export function VaultAddEntry(props: {
  busy: boolean;
  project: string | null;
  act: VaultAct;
  onClose: () => void;
  list: VaultListing;
}) {
  const [type, setType] = useState<"secret" | "login" | "mcp">("secret");
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [site, setSite] = useState("");
  const [username, setUsername] = useState("");
  const [permission, setPermission] = useState<EntryPermission | "default">("default");
  const generator = useGeneratorOptions();
  const login = type === "login";
  let origin = site.trim();
  try {
    origin = new URL(origin.includes("://") ? origin : `https://${origin}`).origin;
  } catch {
    /* Core validates the address. */
  }
  const duplicate = login
    ? props.list.logins.some(
        (l) => l.origin.toLowerCase() === origin.toLowerCase() && l.username === username.trim(),
      )
    : props.list.secrets.some((s) => s.name.toLowerCase() === name.trim().toLowerCase());
  const valid = name.trim() !== "" && (!login || site.trim() !== "") && !duplicate;
  const save = (generate: boolean) => {
    if (!valid || props.busy) return;
    const label = type === "mcp" ? { label: "mcp" } : {};
    const base = { name: name.trim() };
    const project = permission === "default" ? null : props.project;
    let request: SecretsRequest;
    if (login) {
      const fields = { ...base, url: site.trim(), username: username.trim() };
      if (generate)
        request =
          project === null
            ? { op: "generateLogin", ...fields, options: generator.options }
            : {
                op: "generateLoginWithPermission",
                ...fields,
                options: generator.options,
                project,
                permission: permission as EntryPermission,
              };
      else
        request =
          project === null
            ? { op: "addLogin", ...fields, password: value }
            : {
                op: "addLoginWithPermission",
                ...fields,
                password: value,
                project,
                permission: permission as EntryPermission,
              };
    } else {
      if (generate)
        request =
          project === null
            ? { op: "generate", ...base, ...label, options: generator.options }
            : {
                op: "generateWithPermission",
                ...base,
                ...label,
                options: generator.options,
                project,
                permission: permission as EntryPermission,
              };
      else
        request =
          project === null
            ? { op: "add", ...base, ...label, value }
            : {
                op: "addWithPermission",
                ...base,
                ...label,
                value,
                project,
                permission: permission as EntryPermission,
              };
    }
    setValue("");
    props.act(
      request,
      generate ? "Generated and stored. Copied. Clears in 30 s." : "Entry saved.",
      props.onClose,
    );
  };
  return (
    <form
      className="secrets-add vault-add-entry"
      data-setting="add-entry"
      role="group"
      aria-label="Add entry"
      onSubmit={(event) => {
        event.preventDefault();
        if (valid && value !== "" && (!login || value.length >= 8)) save(false);
      }}
    >
      <div className="key-field">
        <span className="key-field-label">Type</span>
        <Selector
          testId="secrets.vault-add-entry-set-value"
          label="Type"
          value={type}
          options={[
            { value: "secret", label: "Secret" },
            { value: "login", label: "Login" },
            { value: "mcp", label: "MCP key" },
          ]}
          onChange={(value: typeof type) => {
            setValue("");
            setType(value);
          }}
        />
      </div>
      <div className="key-field">
        <label htmlFor="entry-name">Name</label>
        <div className="key-field-row">
          <input
            data-testid="secrets.vault-add-entry-set-name"
            id="entry-name"
            autoFocus
            value={name}
            autoComplete="off"
            maxLength={80}
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </div>
      </div>
      {login && (
        <>
          <div className="key-field">
            <label htmlFor="entry-site">Site</label>
            <div className="key-field-row">
              <input
                data-testid="secrets.vault-add-entry-set-site"
                id="entry-site"
                value={site}
                maxLength={2000}
                autoComplete="off"
                onChange={(e) => {
                  setSite(e.target.value);
                }}
              />
            </div>
          </div>
          <div className="key-field">
            <label htmlFor="entry-user">User name</label>
            <div className="key-field-row">
              <input
                data-testid="secrets.vault-add-entry-set-username"
                id="entry-user"
                value={username}
                maxLength={200}
                autoComplete="off"
                onChange={(e) => {
                  setUsername(e.target.value);
                }}
              />
            </div>
          </div>
        </>
      )}
      <SecretField
        testId="secrets.vault-add-entry-secretfield"
        label={login ? "Password" : "Value"}
        value={value}
        onChange={setValue}
        autoComplete="off"
      />
      {duplicate && (
        <p className="key-problem" role="alert">
          This entry already exists. Expand it and choose Replace.
        </p>
      )}
      {props.project !== null && (
        <div className="key-field">
          <span className="key-field-label">Project permission</span>
          <Selector
            testId="secrets.vault-add-entry-selector"
            label="Project permission"
            value={permission}
            disabled={props.busy || !props.list.unlocked}
            options={[
              {
                value: "default",
                label: `Default (${term("vaultPermissions", "askEachTime").name})`,
              },
              ...PERMISSION_OPTIONS,
            ]}
            onChange={setPermission}
          />
        </div>
      )}
      <GeneratorOptions
        scope={`add-${login ? "login" : "secret"}`}
        options={generator.options}
        setOptions={generator.setOptions}
      >
        <Button
          data-testid="secrets.vault-add-entry-tb-btn"
          type="submit"
          variant="primary"
          disabled={props.busy || !valid || value === "" || (login && value.length < 8)}
        >
          Save entry
        </Button>
        <Button
          data-testid="secrets.vault-add-entry-save"
          type="button"

          disabled={props.busy || !valid || !generator.valid}
          onClick={() => {
            save(true);
          }}
        >
          Generate
        </Button>
        <Button
          data-testid="secrets.vault-add-entry-set-value-2"
          type="button"

          disabled={props.busy}
          onClick={() => {
            setValue("");
            props.onClose();
          }}
        >
          Cancel
        </Button>
      </GeneratorOptions>
    </form>
  );
}
