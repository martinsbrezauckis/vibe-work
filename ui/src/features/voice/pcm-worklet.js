/*
 * Audio worklet of the dictation capture: copies each 128-frame block of the first input
 * channel to the main thread. Loaded from the app's own origin (`script-src 'self'`) by `pcmCapture.ts`
 * with `audioWorklet.addModule`; it never sends anything anywhere else.
 */
/* global AudioWorkletProcessor, registerProcessor */
class VwPcmTap extends AudioWorkletProcessor {
  process(inputs) {
    const channel = inputs[0] && inputs[0][0];
    if (channel && channel.length > 0) this.port.postMessage(channel.slice(0));
    return true;
  }
}
registerProcessor("vw-pcm-tap", VwPcmTap);
