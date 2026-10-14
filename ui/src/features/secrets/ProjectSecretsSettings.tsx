/**
 * Settings Vault: one filter/search/action row and one expandable metadata list. Depends on shared
 * Settings controls, key state and project-scoped Vault state. No stored value is read by this page.
 */
import { useState } from "react";
import { Button } from "../../components/Controls";
import { Selector } from "../../components/Selector";
import { TEXT } from "../../vocabulary";
import { useKeyUi } from "../setup/useKeys";
import { KeyNotice } from "../shell/KeyNotice";
import { useShellState } from "../shell/hooks";
import { isMcp } from "./McpLabel";
import { MistralKeyRow } from "./MistralKeySettings";
import { VaultAddEntry } from "./VaultAddEntry";
import { entryAddress, VaultEntryRow, type VaultEntry } from "./VaultEntryRow";
import { VaultLockControls } from "./VaultLockControls";
import { useVaultEntries } from "./vaultEntries";

type Filter = "All" | "Secrets" | "Logins" | "MCP" | "Mistral key";
const FILTERS: readonly Filter[] = ["All", "Secrets", "Logins", "MCP", "Mistral key"];

/** Dense Settings list with live search across every visible entry type. */
export function ProjectSecretsSettings() {
  const {
    core,
    project,
    projectFolders,
    setProject,
    chooseFolder,
    list,
    blocked,
    busy,
    error,
    notice,
    act,
  } = useVaultEntries();
  const target = useShellState((s) => s.settings.target);
  const keyLabel = useKeyUi((s) => s.status?.activeKey?.label ?? "");
  const [filter, setFilter] = useState<Filter>("All");
  const [query, setQuery] = useState("");
  const [adding, setAdding] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [seenTarget, setSeenTarget] = useState<string | null>(null);
  // Navigation changes the view before rendering it; clearing the Settings flash keeps the disclosure open.
  if (target !== seenTarget) {
    setSeenTarget(target);
    if (target === "vault-mistral-key") {
      setExpanded("model");
      setFilter("All");
      setQuery("");
    } else if (target === "add-entry") {
      setAdding(true);
    }
  }
  if (core === null) return null;
  const needle = query.trim().toLocaleLowerCase();
  const entries: VaultEntry[] =
    list?.unlocked === true
      ? [
          ...list.secrets.map((item): VaultEntry => ({ kind: "secret", item })),
          ...list.logins.map((item): VaultEntry => ({ kind: "login", item })),
        ]
      : [];
  const shown = entries
    .filter((entry) => {
      const matchesType =
        filter === "All" ||
        (filter === "Logins" && entry.kind === "login") ||
        (entry.kind === "secret" &&
          (filter === "MCP" ? isMcp(entry.item) : filter === "Secrets" && !isMcp(entry.item)));
      const metadata = [
        entry.item.name,
        list === null ? "" : entryAddress(entry, list),
        entry.kind === "login" ? entry.item.username : "",
        entry.item.own ? "" : "other app",
        entry.kind === "secret" ? (entry.item.label ?? "") : "",
        ...(entry.kind === "secret"
          ? (list?.serverGrants ?? [])
              .filter((g) => g.secretId === entry.item.id)
              .flatMap((g) => [g.server, g.address])
          : []),
      ];
      return matchesType && metadata.some((text) => text.toLocaleLowerCase().includes(needle));
    })
    .sort((a, b) => a.item.name.localeCompare(b.item.name));
  const showModel =
    (filter === "All" || filter === "Mistral key") &&
    ["Mistral key", "model", keyLabel].some((text) => text.toLocaleLowerCase().includes(needle));
  return (
    <section
      className="set-group vault-page"
      data-setting="project-secrets"
      data-toast-avoid="true"
      aria-label="Vault entries"
    >
      <div className="secrets-toolbar vault-toolbar">
        <Selector
          label="Entry type"
          value={filter}
          options={FILTERS.map((type) => ({ value: type, label: type }))}
          onChange={setFilter}
        />
        <div className="set-search">
          <input
            type="search"
            aria-label="Search entries"
            placeholder="Search entries"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape" && query !== "") {
                e.preventDefault();
                e.stopPropagation();
                setQuery("");
              }
            }}
          />
        </div>
        <Button
          type="button"
          variant="primary"
          disabled={busy || list?.unlocked !== true}
          aria-expanded={adding}
          onClick={() => {
            setAdding(!adding);
          }}
        >
          + Add entry
        </Button>
        <VaultLockControls mode="agent" display="icon" />
        <VaultLockControls mode="vault" display="icon" />
      </div>
      <details className="vault-permission-scope-disclosure">
        <summary>Project exceptions</summary>
        <div className="secrets-toolbar vault-permission-scope" aria-label="Permission scope">
          <span>Permission project</span>
          <Selector
            label="Permission project"
            value={project ?? ""}
            disabled={busy || blocked}
            options={[
              ...(project === null
                ? [{ value: "", label: "Choose a project", disabled: true }]
                : []),
              ...projectFolders.map((folder) => ({
                value: folder,
                label: folder,
                tooltip: folder,
              })),
            ]}
            onChange={setProject}
            title={project ?? TEXT.permissionNeedsProject}
          />
          <Button type="button" small disabled={busy || blocked} onClick={chooseFolder}>
            Choose folder...
          </Button>
        </div>
      </details>
      {adding && list?.unlocked === true && (
        <VaultAddEntry
          busy={busy}
          project={project}
          list={list}
          act={act}
          onClose={() => {
            setAdding(false);
          }}
        />
      )}
      {blocked && <KeyNotice inSettings requirement="vault" />}
      {notice !== null && (
        <p className="key-line" role="status">
          {notice}
        </p>
      )}
      {list?.agentAccessLocked === true && (
        <p className="key-line" role="status">
          {TEXT.agentPausedNotice}
        </p>
      )}
      <div className="vault-entry-list" aria-label="Entries">
        {showModel && (
          <MistralKeyRow
            expanded={expanded === "model"}
            onExpandedChange={(open) => {
              setExpanded(open ? "model" : null);
            }}
          />
        )}
        {list !== null &&
          shown.map((entry) => (
            <VaultEntryRow
              key={entry.item.id}
              entry={entry}
              list={list}
              project={project}
              busy={busy}
              expanded={expanded === entry.item.id}
              onExpandedChange={() => {
                setExpanded(expanded === entry.item.id ? null : entry.item.id);
              }}
              act={act}
            />
          ))}
        {!showModel && shown.length === 0 && (
          <p className="key-line" role="status">
            No matching entries.
          </p>
        )}
      </div>
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
