/**
 * Hands the diff line comments' controller to the hunk rows and the panel without threading a prop
 * through the existing Changes components. Without a provider (the controller needs a core) rows render as
 * before, with no comment buttons.
 * Depends on: lineComments.ts. Called by: register.tsx (provider), HunkLines, LineCommentsPanel.
 */
import { createContext, useContext } from "react";

import type { LineComments } from "./lineComments";

const Context = createContext<LineComments | null>(null);

/** Provides the controller. */
export const LineCommentsProvider = Context.Provider;

/** The controller, or `null` when comments are not available. */
export function useLineComments(): LineComments | null {
  return useContext(Context);
}
