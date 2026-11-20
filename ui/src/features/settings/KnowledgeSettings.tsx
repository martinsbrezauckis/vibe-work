/**
 * Settings > Knowledge base: the places the user names so the agent looks there first ("look at our library"), in
 * two groups: the user's Mistral Libraries, and folders on this computer or the user's cloud drives. Add, Rename,
 * Edit note and Remove; the core checks and saves each change, and the page follows changes made elsewhere.
 * Depends on: useKnowledge, KnowledgeForms (the field and the Add forms), Controls, AppContext. Called by:
 * SettingsPage ("knowledge" category).
 * Invariants: names, paths, notes and library names are the user's text and render as text; a place is a pointer
 * only (the copy says so); the Mistral Libraries list is fetched only when the user opens Add in that group; the
 * core's refusal is said once by the app's announcer (the paragraph is no live region) and goes when an Add form
 * closes; an opened Rename or Edit note form takes focus on its field and, closed, hands focus back to the button
 * that opened it.
 */
import { useEffect, useRef, useState } from "react";
import type { KnowledgePlace } from "../../protocol/core";
import { Button } from "../../components/Controls";
import { announce } from "../../state/announcer";
import { useKnowledge } from "../../state/useKnowledge";
import { AddLibrary, AddLocal, Field, type Knowledge } from "./KnowledgeForms";
import { useApp } from "../shell/AppContext";

/** Where a place is, in words. */
function where(place: KnowledgePlace): string {
  switch (place.kind) {
    case "folder":
      return place.location;
    case "drive":
      return `Cloud drive, through your ${place.location} connector`;
    default:
      return `Mistral Library ${place.label ?? place.location}`;
  }
}

/** Which one-line edit a row has open: its name or its note. */
type Editing = { what: "name" | "note"; text: string };

/** One place: its name, where it is and its note, with Rename, Edit note and Remove. */
function PlaceRow({ place, knowledge }: { place: KnowledgePlace; knowledge: Knowledge }) {
  const [editing, setEditing] = useState<Editing | null>(null);
  const row = useRef<HTMLLIElement>(null);
  // The button whose form just closed gets focus back (the form replaced it in the row).
  const closed = useRef<Editing["what"] | null>(null);
  const mode = editing?.what ?? null;
  useEffect(() => {
    const root = row.current;
    if (root === null) return;
    if (mode !== null) {
      root.querySelector<HTMLElement>("input")?.focus();
      return;
    }
    if (closed.current === null) return;
    const opener =
      closed.current === "name" ? "settings.knowledge-rename" : "settings.knowledge-note-edit";
    root.querySelector<HTMLElement>(`[data-testid="${opener}"]`)?.focus();
    closed.current = null;
  }, [mode]);
  const close = () => {
    if (editing !== null) closed.current = editing.what;
    setEditing(null);
  };
  if (editing !== null) {
    const name = editing.what === "name";
    const prefix = name ? "settings.knowledge-rename" : "settings.knowledge-note";
    return (
      <li className="knowledge-place" data-place={place.id} data-settings-form="" ref={row}>
        <Field
          label={name ? `New name for ${place.name}` : `Note for ${place.name}`}
          value={editing.text}
          onChange={(text) => {
            setEditing({ ...editing, text });
          }}
          testId={`${prefix}-field`}
          {...(name ? {} : { placeholder: "contracts and offers" })}
        />
        <div className="knowledge-actions">
          <Button
            data-testid={`${prefix}-save`}
            shape="outlined"
            disabled={knowledge.busy}
            onClick={() => {
              const saved = name
                ? knowledge.rename(place.id, editing.text)
                : knowledge.setNote(place.id, editing.text);
              void saved.then((ok) => {
                if (ok) close();
              });
            }}
          >
            Save
          </Button>
          <Button data-testid={`${prefix}-cancel`} shape="outlined" onClick={close}>
            Cancel
          </Button>
        </div>
      </li>
    );
  }
  return (
    <li className="knowledge-place" data-place={place.id} ref={row}>
      <span className="knowledge-place-text">
        <span className="knowledge-place-name">{place.name}</span>
        <span className="d">{where(place)}</span>
        {place.note !== undefined && <span className="d">{place.note}</span>}
      </span>
      <div className="knowledge-actions">
        <Button
          data-testid="settings.knowledge-rename"
          shape="outlined"
          aria-label={`Rename ${place.name}`}
          disabled={knowledge.busy}
          onClick={() => {
            setEditing({ what: "name", text: place.name });
          }}
        >
          Rename
        </Button>
        <Button
          data-testid="settings.knowledge-note-edit"
          shape="outlined"
          aria-label={`Edit note for ${place.name}`}
          disabled={knowledge.busy}
          onClick={() => {
            setEditing({ what: "note", text: place.note ?? "" });
          }}
        >
          Edit note
        </Button>
        <Button
          data-testid="settings.knowledge-remove"
          shape="outlined"
          aria-label={`Remove ${place.name}`}
          disabled={knowledge.busy}
          onClick={() => {
            void knowledge.remove(place.id);
          }}
        >
          Remove
        </Button>
      </div>
    </li>
  );
}

function PlaceList({
  places,
  knowledge,
  empty,
}: {
  places: KnowledgePlace[];
  knowledge: Knowledge;
  empty: string;
}) {
  if (places.length === 0) return <p className="key-line">{empty}</p>;
  return (
    <ul className="knowledge-list">
      {places.map((place) => (
        <PlaceRow key={place.id} place={place} knowledge={knowledge} />
      ))}
    </ul>
  );
}

/** The Knowledge base page. */
export function KnowledgeSettings() {
  const { core } = useApp();
  const knowledge = useKnowledge(core);
  const [adding, setAdding] = useState<"library" | "local" | null>(null);
  // A closed Add form (Add or Cancel) hands focus back to the button that opened it.
  const section = useRef<HTMLElement>(null);
  const closedForm = useRef<"library" | "local" | null>(null);
  useEffect(() => {
    if (adding !== null || closedForm.current === null) return;
    section.current
      ?.querySelector<HTMLElement>(`[data-testid="settings.knowledge-add-${closedForm.current}"]`)
      ?.focus();
    closedForm.current = null;
  }, [adding]);
  // A refusal is said once, assertively, by the app's announcer; the paragraph below only shows it.
  const refusal = knowledge.error;
  useEffect(() => {
    if (refusal !== null) announce(refusal, { assertive: true });
  }, [refusal]);
  if (core === null) return null;
  const places = knowledge.places ?? [];
  const cloud = places.filter((p) => p.kind === "mistralLibrary");
  const local = places.filter((p) => p.kind !== "mistralLibrary");
  return (
    <section aria-labelledby="knowledge-title" className="knowledge" ref={section}>
      <h3 id="knowledge-title">Knowledge base</h3>
      <p className="key-field-hint ui-help">
        Name the places that hold your knowledge. When you say “look at our library”, the agent
        looks there first instead of searching the whole computer. This list only points the way: it
        gives the agent no extra access, and nothing is copied. A new document is saved into a place
        only when you ask for it.
      </p>
      <div className="knowledge-group" data-setting="knowledge-mistral">
        <h4>In your Mistral account</h4>
        <PlaceList places={cloud} knowledge={knowledge} empty="No Mistral Libraries added." />
        {adding === "library" ? (
          <AddLibrary
            knowledge={knowledge}
            onDone={() => {
              closedForm.current = "library";
              knowledge.clearError();
              setAdding(null);
            }}
          />
        ) : (
          <Button
            data-testid="settings.knowledge-add-library"
            shape="outlined"
            disabled={knowledge.places === null}
            onClick={() => {
              setAdding("library");
              void knowledge.loadLibraries();
            }}
          >
            Add a library
          </Button>
        )}
      </div>
      <div className="knowledge-group" data-setting="knowledge-local">
        <h4>On this computer and your drives</h4>
        <PlaceList places={local} knowledge={knowledge} empty="No folders or drives added." />
        {adding === "local" ? (
          <AddLocal
            knowledge={knowledge}
            onDone={() => {
              closedForm.current = "local";
              knowledge.clearError();
              setAdding(null);
            }}
          />
        ) : (
          <Button
            data-testid="settings.knowledge-add-local"
            shape="outlined"
            disabled={knowledge.places === null}
            onClick={() => {
              setAdding("local");
            }}
          >
            Add a folder or drive
          </Button>
        )}
      </div>
      {knowledge.error !== null && <p className="key-problem">{knowledge.error}</p>}
    </section>
  );
}
