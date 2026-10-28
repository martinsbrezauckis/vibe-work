/**
 * The new-chat view's composer key (New chat). The view has a real composer before any
 * session exists; its draft, images, chips and held picks live in the composer store under this key, so they
 * persist like any draft and survive switching away and back. The session is created on the first send
 * (`state/composer.ts`, `sendNewChat`).
 * Depends on: nothing. Called by: the composer store and facade, the new-chat view, the `ui/composer` and
 * `ui/rail` routes.
 * Invariant: never a real session id (Vibe's ids are UUIDs), so no core call is ever made for it.
 */

/** The composer key of the new-chat view. */
export const NEW_CHAT = "new-chat";
