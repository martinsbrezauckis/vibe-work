/**
 * Toast notifications: a list in a store. Placement (never over an input) is decided by the view
 * (`features/shell/toastPlacement.ts`); this module only holds what is shown.
 * Depends on: createStore, notifications types. Called by: coreStore (pushes), the Toasts view
 * (reads, dismisses), tests.
 * Invariants: ids are unique; active toasts are bounded separately from the full-text history.
 * Dismissal/expiry never removes retained text. Only allowed bell kinds enter memory-only history.
 */
import { createStore } from "./createStore";
import { isBellKind, type ToastSpec } from "./notifications";

/** Most toasts kept at once. */
export const MAX_TOASTS = 4;
/** Full notifications retained for this app run, including expired and OS-only notices. */
export const MAX_NOTIFICATIONS = 100;

/** A toast in the list. */
export interface Toast extends ToastSpec {
  id: number;
  /** Local receipt time, kept with the complete text for the app run. */
  createdAt: number;
  /** Read when the retained panel is opened, including notices received while it is open. */
  read: boolean;
}

/** Creates a toast list. */
export function createToasts() {
  const store = createStore<readonly Toast[]>([]);
  const history = createStore<readonly Toast[]>([]);
  const panel = createStore<{ open: boolean; selectedId: number | null }>({
    open: false,
    selectedId: null,
  });
  let next = 1;
  const markRead = () => {
    history.set((list) => list.map((notice) => (notice.read ? notice : { ...notice, read: true })));
  };
  const retain = (notice: Toast): void => {
    if (isBellKind(notice.bellKind))
      history.set((list) => [...list, notice].slice(-MAX_NOTIFICATIONS));
  };
  const make = (spec: ToastSpec): Toast => ({
    ...spec,
    id: next++,
    createdAt: Date.now(),
    read: panel.get().open,
  });
  const record = (spec: ToastSpec): number => {
    const notice = make(spec);
    retain(notice);
    return notice.id;
  };
  const open = (id?: number): void => {
    const active = store.get().find((notice) => notice.id === id);
    if (active !== undefined && !history.get().some((notice) => notice.id === id)) {
      retain(active);
    }
    markRead();
    panel.set({ open: true, selectedId: id ?? null });
  };
  return {
    store,
    history,
    panel,
    /** Retains full text without displaying an in-app toast (used for OS notifications). */
    record,
    /** Opens the full notifications panel, optionally at one retained notification. */
    open,
    /** Opens the most recent notification for a session after OS activation. */
    openSession(sessionId: string): void {
      open([...history.get()].reverse().find((notice) => notice.sessionId === sessionId)?.id);
    },
    /** Closes the panel while retaining its messages. */
    close(): void {
      panel.set((state) => ({ ...state, open: false }));
    },
    /** Removes a retained entry and its active toast when dismissed from the panel. */
    forget(id: number): void {
      store.set((list) => list.filter((toast) => toast.id !== id));
      history.set((list) => list.filter((toast) => toast.id !== id));
      panel.set((state) => (state.selectedId === id ? { ...state, selectedId: null } : state));
    },
    /** Dismisses all retained and active entries, leaving the panel open at its empty state. */
    clear(): void {
      store.set([]);
      history.set([]);
      panel.set((state) => ({ ...state, selectedId: null }));
    },
    /** Adds a toast and returns its id. */
    push(spec: ToastSpec): number {
      const notice = make(spec);
      retain(notice);
      store.set((list) => [...list, notice].slice(-MAX_TOASTS));
      return notice.id;
    },
    /** Removes one toast. */
    dismiss(id: number): void {
      store.set((list) => list.filter((t) => t.id !== id));
    },
  };
}

/** The toast list the app uses. */
export type Toasts = ReturnType<typeof createToasts>;
