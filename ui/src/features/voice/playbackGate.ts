/** Mutes every app Web Audio player during permission capture, including sources decoded after capture starts. */
let holds = 0;
const outputs = new Set<GainNode>();

export function playbackOutput(context: AudioContext): { node: GainNode; release: () => void } {
  const node = context.createGain();
  node.gain.value = holds > 0 ? 0 : 1;
  node.connect(context.destination);
  outputs.add(node);
  return {
    node,
    release: () => {
      outputs.delete(node);
      node.disconnect();
    },
  };
}

/** Restore output only after every concurrent permission capture has released it. */
export function quietPlayback(): () => void {
  holds++;
  for (const node of outputs) node.gain.value = 0;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    holds--;
    if (holds === 0) for (const node of outputs) node.gain.value = 1;
  };
}
