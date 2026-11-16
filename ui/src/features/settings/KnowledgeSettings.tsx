/**
 * Settings > Knowledge base: the places the user names so the agent looks there first ("look at our library"), in
 * two groups: the user's Mistral Libraries, and folders on this computer or the user's cloud drives. Add, Rename and
 * Remove; the core checks and saves each change.
 * Depends on: useKnowledge, KnowledgeForms (the field and the Add forms), Controls, AppContext. Called by:
 * SettingsPage ("knowledge" category).
 * Invariants: names, paths, notes and library names are the user's text and render as text; a place is a pointer
 * only (the copy says so); the Mistral Libraries list is fetched only when the user opens Add in that group; the
 * core's refusal is said once by the app's announcer (the paragraph is no live region) and goes when an Add form
 * closes.
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

/** One place: its name and where it is, with Rename and Remove. */
function PlaceRow({ place, knowledge }: { place: KnowledgePlace; knowledge: Knowledge }) {
  const [draft, setDraft] = useState<string | null>(null);
  if (draft !== null)
    return (
      <li className="knowledge-place" data-place={place.id} data-settings-form="">
        <Field
          label={`New name for ${place.name}`}
          value={draft}
          onChange={setDraft}
          testId="settings.knowledge-rename-field"
        />
        <div className="knowledge-actions">
          <Button
            data-testid="settings.knowledge-rename-save"
            shape="outlined"
            disabled={knowledge.busy}
            onClick={() => {
              void knowledge.rename(place.id, draft).then((ok) => {
                if (ok) setDraft(null);
              });
            }}
          >
            Save
          </Button>
          <Button
            data-testid="settings.knowledge-rename-cancel"
            shape="outlined"
            onClick={() => {
              setDraft(null);
            }}
          >
            Cancel
          </Button>
        </div>
      </li>
    );
  return (
    <li className="knowledge-place" data-place={place.id}>
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
            setDraft(place.name);
          }}
        >
          Rename
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
        Name the places that hold your knowledge. When you say &quot;look at our library&quot;, the
        agent looks there first instead of searching the whole computer. This list only points the
        way: it gives the agent no extra access, and nothing is copied. A new document is saved into
        a place only when you ask for it.
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
