/**
 * Settings > Knowledge base forms: the shared one-line field and the two Add forms (a Mistral library by name, a
 * folder or cloud-drive location on this computer). The core checks and saves each place.
 * Depends on: useKnowledge, Controls, Selector, AppContext. Called by: KnowledgeSettings.
 * Invariants: names, paths, notes and library names are the user's text and render as text; the Mistral Libraries
 * list is fetched only when the user opens Add in that group, the connectors only when the user picks a cloud drive
 * that is not on this computer (free text only when the install has none or they cannot be read); a failed list is
 * said once by the app's announcer; an opened Add form takes focus on its first control; an edit in an Add form
 * clears the shown refusal.
 */
import { type ReactElement, useEffect, useId, useRef, useState } from "react";
import { Button } from "../../components/Controls";
import { Selector } from "../../components/Selector";
import { announce } from "../../state/announcer";
import { type NewPlace, useKnowledge } from "../../state/useKnowledge";
import { useApp } from "../shell/AppContext";
import { ACT_ADD, ACT_CANCEL, ACT_CHOOSE_FOLDER } from "./actionLabels";

/** The `useKnowledge` result the Knowledge base forms read and act on. */
export type Knowledge = ReturnType<typeof useKnowledge>;

/** One labelled one-line text field on the shared key-field style. */
export function Field(props: {
  label: string;
  value: string;
  onChange: (text: string) => void;
  testId: string;
  placeholder?: string;
  extra?: ReactElement;
}): ReactElement {
  const id = useId();
  return (
    <div className="key-field">
      <label htmlFor={id}>{props.label}</label>
      <div className="key-field-row">
        <input
          id={id}
          data-testid={props.testId}
          type="text"
          autoComplete="off"
          spellCheck={false}
          value={props.value}
          placeholder={props.placeholder}
          onChange={(e) => {
            props.onChange(e.target.value);
          }}
        />
        {props.extra}
      </div>
    </div>
  );
}

/** Focuses the first control of a just-opened form (`ref`), so the keyboard continues inside it. */
function useFocusFirst(ref: React.RefObject<HTMLDivElement | null>): void {
  useEffect(() => {
    ref.current
      ?.querySelector<HTMLElement>("input:not([disabled]), button:not([disabled])")
      ?.focus();
  }, [ref]);
}

/** `set` that first clears the shown refusal: editing the form answers it. */
function clearing<T>(knowledge: Knowledge, set: (value: T) => void): (value: T) => void {
  return (value) => {
    knowledge.clearError();
    set(value);
  };
}

/** Add form of the Mistral group: pick one of the user's libraries, give it a name. */
export function AddLibrary({ knowledge, onDone }: { knowledge: Knowledge; onDone: () => void }) {
  const [library, setLibrary] = useState("");
  const [name, setName] = useState("");
  const chosen = knowledge.libraries?.find((l) => l.id === library);
  const form = useRef<HTMLDivElement>(null);
  useFocusFirst(form);
  const listError = knowledge.librariesError;
  useEffect(() => {
    if (listError !== null) announce(listError, { assertive: true });
  }, [listError]);
  return (
    <div
      className="knowledge-add"
      data-settings-form=""
      data-testid="settings.knowledge-add-library-form"
      ref={form}
    >
      {knowledge.librariesError !== null && (
        <p className="key-problem">{knowledge.librariesError}</p>
      )}
      {knowledge.libraries === null && knowledge.librariesError === null && (
        <p className="key-line" role="status">
          Loading your libraries…
        </p>
      )}
      {knowledge.libraries !== null && (
        <Selector
          testId="settings.knowledge-library"
          label="Library"
          value={library}
          emptyLabel="You have no Mistral Libraries yet. Make one in Libraries."
          options={[
            ...(library === "" ? [{ value: "", label: "Choose a library", disabled: true }] : []),
            ...knowledge.libraries.map((l) => ({ value: l.id, label: l.name })),
          ]}
          onChange={(value) => {
            knowledge.clearError();
            setLibrary(value);
            if (name === "") setName(knowledge.libraries?.find((l) => l.id === value)?.name ?? "");
          }}
        />
      )}
      <Field
        label="Name you use for it"
        value={name}
        onChange={clearing(knowledge, setName)}
        testId="settings.knowledge-library-name"
        placeholder="our research"
      />
      <div className="knowledge-actions">
        <Button
          data-testid="settings.knowledge-add-library-save"
          shape="outlined"
          variant="primary"
          disabled={knowledge.busy || chosen === undefined}
          onClick={() => {
            if (chosen === undefined) return;
            void knowledge
              .add({ name, kind: "mistralLibrary", location: chosen.id, label: chosen.name })
              .then((ok) => {
                if (ok) onDone();
              });
          }}
        >
          {ACT_ADD}
        </Button>
        <Button
          data-testid="settings.knowledge-add-library-cancel"
          shape="outlined"
          onClick={onDone}
        >
          {ACT_CANCEL}
        </Button>
      </div>
    </div>
  );
}

/**
 * The connector of an online-only cloud drive: a pick of the connectors this install has, or, when it has none or
 * they cannot be read, the connector's name typed in a field that says why.
 */
function ConnectorPick({
  knowledge,
  value,
  onChange,
}: {
  knowledge: Knowledge;
  value: string;
  onChange: (name: string) => void;
}): ReactElement {
  const choices = knowledge.connectors;
  if (choices === null && !knowledge.connectorsFailed)
    return (
      <p className="key-line" role="status">
        Loading your connectors…
      </p>
    );
  if (choices === null || choices.length === 0)
    return (
      <>
        <Field
          label="Connector name"
          value={value}
          onChange={onChange}
          testId="settings.knowledge-connector"
          placeholder="Google Drive"
        />
        <p className="key-field-hint ui-help">
          {choices === null
            ? "Your connectors could not be read. Type the name of the connector you use for this drive."
            : "No connectors are set up yet. Type the name of the connector you will use for this drive, or add it in Connectors first."}
        </p>
      </>
    );
  return (
    <Selector
      testId="settings.knowledge-connector-pick"
      label="Connector"
      value={value}
      options={[
        ...(value === "" ? [{ value: "", label: "Choose a connector", disabled: true }] : []),
        ...choices.map((c) => ({ value: c.name, label: c.name, description: c.status })),
      ]}
      onChange={onChange}
    />
  );
}

/** Add form of the local group: a folder (also a synced cloud drive folder) or an online-only cloud drive. */
export function AddLocal({ knowledge, onDone }: { knowledge: Knowledge; onDone: () => void }) {
  const { core } = useApp();
  const [kind, setKind] = useState<"folder" | "drive">("folder");
  const [name, setName] = useState("");
  const [location, setLocation] = useState("");
  const [note, setNote] = useState("");
  const place: NewPlace = { name, kind, location, ...(note.trim() === "" ? {} : { note }) };
  const form = useRef<HTMLDivElement>(null);
  useFocusFirst(form);
  return (
    <div
      className="knowledge-add"
      data-settings-form=""
      data-testid="settings.knowledge-add-local-form"
      ref={form}
    >
      <Selector
        testId="settings.knowledge-kind"
        label="Kind"
        value={kind}
        options={[
          {
            value: "folder",
            label: "Folder on this computer",
            description: "Also a cloud drive folder synced to this computer",
          },
          {
            value: "drive",
            label: "Cloud drive not on this computer",
            description: "Reached through your connector for that drive",
          },
        ]}
        onChange={(value) => {
          knowledge.clearError();
          setKind(value);
          setLocation("");
          if (value === "drive" && knowledge.connectors === null) void knowledge.loadConnectors();
        }}
      />
      <Field
        label="Name you use for it"
        value={name}
        onChange={clearing(knowledge, setName)}
        testId="settings.knowledge-name"
        placeholder="our library"
      />
      {kind === "folder" ? (
        <Field
          label="Folder"
          value={location}
          onChange={clearing(knowledge, setLocation)}
          testId="settings.knowledge-folder"
          extra={
            <Button
              data-testid="settings.knowledge-choose-folder"
              shape="outlined"
              disabled={core === null}
              onClick={() => {
                void core?.client
                  .pickFolder("knowledge")
                  .then((folder) => {
                    if (folder !== null) setLocation(folder);
                  })
                  .catch(() => undefined);
              }}
            >
              {ACT_CHOOSE_FOLDER}
            </Button>
          }
        />
      ) : (
        <ConnectorPick
          knowledge={knowledge}
          value={location}
          onChange={clearing(knowledge, setLocation)}
        />
      )}
      <Field
        label="Note for the agent (optional)"
        value={note}
        onChange={clearing(knowledge, setNote)}
        testId="settings.knowledge-note"
        placeholder="contracts and offers"
      />
      <div className="knowledge-actions">
        <Button
          data-testid="settings.knowledge-add-local-save"
          shape="outlined"
          variant="primary"
          disabled={knowledge.busy}
          onClick={() => {
            void knowledge.add(place).then((ok) => {
              if (ok) onDone();
            });
          }}
        >
          {ACT_ADD}
        </Button>
        <Button data-testid="settings.knowledge-add-local-cancel" shape="outlined" onClick={onDone}>
          {ACT_CANCEL}
        </Button>
      </div>
    </div>
  );
}
