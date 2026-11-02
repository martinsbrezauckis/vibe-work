/** A short local PCM chime, encoded as WAV for the existing audio player. No network or audio assets. */
export function promptTone(): Uint8Array {
  const rate = 16000,
    samples = 2400;
  const bytes = new Uint8Array(44 + samples * 2);
  const view = new DataView(bytes.buffer);
  const word = (at: number, value: string) => {
    bytes.set(new TextEncoder().encode(value), at);
  };
  word(0, "RIFF");
  view.setUint32(4, bytes.length - 8, true);
  word(8, "WAVEfmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  word(36, "data");
  view.setUint32(40, samples * 2, true);
  for (let i = 0; i < samples; i++) {
    const envelope = Math.sin((Math.PI * i) / samples) ** 2;
    view.setInt16(
      44 + i * 2,
      Math.round(Math.sin((2 * Math.PI * 660 * i) / rate) * envelope * 5000),
      true,
    );
  }
  return bytes;
}
