/** Wires recall to the shared composer store and typed history command; menus take priority.
 * React only places the caret; navigation decisions and file I/O live in state and the core. */
import {
  useEffect,
  useState,
  useLayoutEffect,
  useRef,
  useSyncExternalStore,
  type RefObject,
  type KeyboardEvent,
} from "react";
import { PromptHistory, syncPromptHistory } from "../../state/promptHistory";
import { useApp } from "../shell/AppContext";

/**
 * Prompt history for the composer of `sessionId`: the shared history entries, a refresh from the core, and a key
 * handler that replaces the draft with an earlier prompt (returns true when it did) with the caret at its end.
 */
export function usePromptHistory(
  sessionId: string,
  draft: string,
  box: RefObject<HTMLTextAreaElement | null>,
) {
  const { core, composer } = useApp();
  const [navigation] = useState(() => new PromptHistory());
  const entries = useSyncExternalStore(
    (listener) => composer.history.subscribe(listener),
    () => composer.history.get(),
  );
  const caret = useRef<number | null>(null);
  useEffect(() => {
    navigation.replace(entries);
  }, [navigation, entries]);
  useEffect(() => {
    navigation.edit();
  }, [navigation, sessionId]);
  const refresh = () => {
    if (core !== null) void syncPromptHistory(core, composer);
  };
  useEffect(() => {
    if (core !== null) void syncPromptHistory(core, composer);
  }, [core, composer, sessionId]);
  useLayoutEffect(() => {
    if (caret.current !== null) {
      box.current?.setSelectionRange(caret.current, caret.current);
      caret.current = null;
    }
  }, [draft, box]);
  return {
    /** The shared history, oldest first (prompt search lists it newest first). */
    entries,
    refresh,
    edit: () => {
      navigation.edit();
    },
    /** Puts a prompt picked in the search into the draft, caret at its end; never sends. */
    load: (text: string) => {
      navigation.edit();
      caret.current = text.length;
      composer.setDraft(sessionId, text);
      box.current?.focus();
    },
    key: (event: KeyboardEvent<HTMLTextAreaElement>) => {
      const input = event.currentTarget;
      const text = navigation.key(
        {
          key: event.key,
          shiftKey: event.shiftKey,
          ctrlKey: event.ctrlKey,
          altKey: event.altKey,
          metaKey: event.metaKey,
          isComposing: event.nativeEvent.isComposing,
        },
        input.value,
        input.selectionStart,
        input.selectionEnd,
      );
      if (text === null) return false;
      event.preventDefault();
      caret.current = text.length;
      composer.setDraft(sessionId, text);
      return true;
    },
  };
}
