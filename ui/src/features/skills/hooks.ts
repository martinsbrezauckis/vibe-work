/** Skills view subscriptions. Depends on AppContext and stores; called by the Skills tab. */
import { useApp } from "../shell/AppContext";
import { createStore, useStore } from "../../state/createStore";
import { OFFERS_INITIAL, type OffersState } from "../../state/installOffers";
import { SKILLS_INITIAL, type SkillsState } from "../../state/skills";
const NONE = createStore<SkillsState>(SKILLS_INITIAL);
const NO_OFFERS = createStore<OffersState>(OFFERS_INITIAL);
export function useSkills<T>(select: (s: SkillsState) => T): T {
  return useStore(useApp().skills?.store ?? NONE, select);
}
/** The skill rows' install offers (see `useStore` for the selector rule). */
export function useSkillOffers<T>(select: (s: OffersState) => T): T {
  return useStore(useApp().skills?.offers ?? NO_OFFERS, select);
}
