/** Fits open file tabs without moving the working tabs; used by the document workspace and tested with overflow choices. */
export function fitFileTabs(
  order: readonly string[],
  previous: readonly string[],
  capacity: number,
  active: string | null,
  dirty: (file: string) => boolean,
  used: (file: string) => number,
): readonly string[] {
  const limit = Math.max(1, capacity);
  const visible = previous.filter((file) => order.includes(file)).slice(0, limit);
  const include = (file: string) => {
    if (visible.includes(file)) return;
    if (visible.length < limit) {
      visible.push(file);
      return;
    }
    const candidates = visible.filter((f) => f !== active);
    const clean = candidates.filter((f) => !dirty(f));
    const victim = (clean.length ? clean : candidates).sort((a, b) => used(a) - used(b))[0];
    if (victim !== undefined) visible[visible.indexOf(victim)] = file;
  };
  for (const file of order.filter(dirty)) include(file);
  if (active !== null && order.includes(active)) include(active);
  for (const file of order) if (visible.length < limit) include(file);
  return visible;
}
