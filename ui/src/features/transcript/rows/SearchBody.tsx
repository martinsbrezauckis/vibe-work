/**
 * The body of a web search step, for both shapes `model/sources.ts` reads (Vibe's built-in
 * `web_search` and Mistral's hosted search connector): the query, the sources (each through the link
 * dialog), and the built-in tool's own answer text when there is one. The connector's raw output (JSON of
 * every page with snippets and favicon URLs) is not shown: nothing is fetched, and the list is the part a
 * person reads.
 * Depends on: model/entries, model/sources, ShowAll, SourcesList. Called by: StepBody.
 * Invariants: render only; all text is plain text nodes.
 */
import type { ReactElement } from "react";

import { type EffectView, type Obj, entryId } from "../model/entries";
import { searchAnswer, searchQuery, searchSources } from "../model/sources";
import { ShowAll } from "./ShowAll";
import { SourcesList } from "./SourcesList";

/** The search step's body. */
export function SearchBody({ entry, view }: { entry: Obj; view: EffectView }): ReactElement {
  const sources = searchSources(entry);
  const answer = searchAnswer(entry);
  return (
    <>
      <p className="search-q">
        Searched for <q>{searchQuery(entry)}</q>
      </p>
      {sources.length > 0 ? (
        <SourcesList sources={sources} label="Sources of this search" />
      ) : view.live ? null : (
        <p className="d">No sources were returned.</p>
      )}
      {answer !== "" ? <ShowAll id={`${entryId(entry)}:answer`} text={answer} /> : null}
    </>
  );
}
