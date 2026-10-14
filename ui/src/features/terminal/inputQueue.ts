/**
 * Keeps a terminal's input in order: one `write` in flight at a time, and whatever is typed while
 * it is in flight goes out as one coalesced write after it. Two concurrent IPC calls may be served
 * in any order by the core's async runtime, so sending each keystroke on its own could reorder them.
 * Depends on: nothing. Called by: session.ts (one queue per mounted terminal), tests.
 * Invariants: bytes leave in the order they were queued; text and binary input are never merged
 * into one write (they are encoded differently); a failed write drops only that write and reports
 * it, the queue keeps going.
 */

/** Sends one write; rejects on failure. */
export type SendInput = (data: string, binary: boolean) => Promise<void>;

/** A per-terminal input queue. */
export interface InputQueue {
  /** Queues text (keystrokes, a paste). */
  text(data: string): void;
  /** Queues binary input (each character one byte, xterm's `onBinary`). */
  binary(data: string): void;
  /** Stops sending; queued input is dropped. */
  close(): void;
}

interface Item {
  data: string;
  binary: boolean;
}

/** Creates a queue that sends through `send` and reports failures to `onError`. */
export function createInputQueue(send: SendInput, onError: (error: unknown) => void): InputQueue {
  const items: Item[] = [];
  let busy = false;
  let closed = false;

  const pump = async (): Promise<void> => {
    if (busy) return;
    busy = true;
    while (!closed) {
      const first = items.shift();
      if (first === undefined) break;
      // Coalesce everything of the same kind queued behind it.
      let data = first.data;
      while (items[0]?.binary === first.binary) data += items.shift()?.data ?? "";
      try {
        await send(data, first.binary);
      } catch (error) {
        onError(error);
      }
    }
    busy = false;
  };

  const push = (data: string, binary: boolean): void => {
    if (closed || data.length === 0) return;
    items.push({ data, binary });
    void pump();
  };

  return {
    text: (data) => {
      push(data, false);
    },
    binary: (data) => {
      push(data, true);
    },
    close() {
      closed = true;
      items.length = 0;
    },
  };
}
