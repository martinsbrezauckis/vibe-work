/** Coalesces Changes refresh bursts per session/repository into one active and one queued read.
 * Called by the Changes store. Each caller awaits its own batch, never later background batches.
 */
export function createReadQueue() {
  type Batch = { reading: Promise<void>; next?: Promise<void> };
  const active = new Map<string, Batch>();
  const begin = (key: string, read: () => Promise<void>): Promise<void> => {
    const batch: Batch = {
      reading: Promise.resolve().then(read),
    };
    active.set(key, batch);
    batch.reading = batch.reading.finally(() => {
      if (active.get(key) === batch) active.delete(key);
    });
    return batch.reading;
  };
  return (key: string, read: () => Promise<void>): Promise<void> => {
    const batch = active.get(key);
    if (batch === undefined) return begin(key, read);
    batch.next ??= batch.reading.then(() => begin(key, read));
    return batch.next;
  };
}
