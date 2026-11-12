// Isolated-world media controls and loaded captions. Host supplies bounded arguments and polls pending operations.
// Handles retain exact elements in this document; detached/replaced media never silently retargets.
(() => {
  // eslint-disable-next-line no-undef -- Expanded by the Rust script assembler.
  const req = __VW_MEDIA_REQUEST__;
  const key = '__vw_page_media_v1';
  const store = window[key] || (window[key] = { elements: new Map(), next: 0, document: crypto.randomUUID().replaceAll('-', ''), pending: new Map() });
  const clip = (text, max = 300) => {
    let result = '';
    for (const character of String(text || '')) {
      if (result.length + character.length > max) break;
      const code = character.charCodeAt(0);
      result += character.length === 1 && code >= 0xd800 && code <= 0xdfff ? '\ufffd' : character;
    }
    return result;
  };
  const error = (reason) => JSON.stringify({ error: reason });
  const forget = (token) => {
    store.pending.get(token)?.cleanup?.();
    store.pending.delete(token);
  };
  const state = (el, mediaId) => ({
    mediaId, kind: el.localName, title: clip(el.title || el.getAttribute('aria-label')),
    paused: el.paused, ended: el.ended, muted: el.muted, volume: el.volume,
    timeMs: Math.round(el.currentTime * 1000),
    durationMs: Number.isFinite(el.duration) ? Math.round(el.duration * 1000) : null,
    readyState: el.readyState, seeking: el.seeking,
    error: el.error ? { code: el.error.code, message: clip(el.error.message) } : null,
  });
  if (req.action === 'media.read') {
    for (const [id, el] of store.elements) if (!el.isConnected) store.elements.delete(id);
    const all = Array.from(document.querySelectorAll('video,audio'));
    const items = all.slice(0, 32).map((el) => {
      let id = Array.from(store.elements).find((entry) => entry[1] === el)?.[0];
      if (!id) { id = `m${store.document}_${++store.next}`; store.elements.set(id, el); }
      return state(el, id);
    });
    return JSON.stringify({ items, truncated: all.length > 32, scope: 'main document' });
  }
  if (req.action === 'media.poll') {
    const pending = store.pending.get(req.token);
    if (!pending) return error('The media operation is no longer available; read media state before trying again.');
    if (!pending.el.isConnected) { forget(req.token); return error('The media element was removed; read media state again.'); }
    if (pending.error) { forget(req.token); return error(pending.error); }
    const result = { state: state(pending.el, pending.id), pending: !pending.done };
    if (pending.done) forget(req.token);
    return JSON.stringify(result);
  }
  const el = store.elements.get(req.mediaId);
  if (!el?.isConnected) return error('The media handle is stale or unknown; use media.read and its mediaId before trying again.');
  if (req.action === 'media.transcript') {
    let remaining = 12000, truncated = el.textTracks.length > 8;
    const tracks = Array.from(el.textTracks).slice(0, 8).map((track) => {
      const cues = [];
      for (const cue of Array.from(track.cues || [])) {
        if (remaining <= 0 || cues.length >= 200) { truncated = true; break; }
        const text = clip(cue.text, Math.min(remaining, 2000));
        if (String(cue.text || "").length > text.length) truncated = true;
        remaining -= text.length;
        cues.push({ startMs: Math.round(cue.startTime * 1000), endMs: Math.round(cue.endTime * 1000), text });
      }
      return { label: clip(track.label), language: clip(track.language, 80), kind: track.kind, cues };
    });
    return JSON.stringify({ state: state(el, req.mediaId), tracks, truncated,
      availability: tracks.some((track) => track.cues.length) ? 'loaded captions' : 'No loaded captions; use the page transcript if available. Audio transcription is unavailable.' });
  }
  try {
    if (req.action === 'media.pause') el.pause();
    else if (req.action === 'media.mute') el.muted = req.muted;
    else if (req.action === 'media.seek') {
      if (!Number.isFinite(el.duration) || req.timeMs / 1000 > el.duration) return error('The requested media time is outside a known duration; read media state and choose a loaded position.');
      if (store.pending.size >= 8) return error('Too many pending media operations; read the media state before trying again.');
      const token = crypto.randomUUID(), pending = { el, id: req.mediaId, done: false, error: null };
      const settled = () => {
        if (el.seeking || el.readyState < 2) return;
        if (Math.abs(el.currentTime * 1000 - req.timeMs) > 250) {
          pending.error = 'The media settled at a different position; read media state and check the page controls before trying again.';
        } else pending.done = true;
      };
      const rejected = () => { pending.error = 'Media seeking failed; read media state and check the source before trying again.'; };
      pending.cleanup = () => {
        el.removeEventListener('seeked', settled);
        el.removeEventListener('loadeddata', settled);
        el.removeEventListener('error', rejected);
      };
      store.pending.set(token, pending);
      el.addEventListener('seeked', settled);
      el.addEventListener('loadeddata', settled);
      el.addEventListener('error', rejected);
      setTimeout(() => forget(token), 10000);
      try { el.currentTime = req.timeMs / 1000; } catch (cause) { forget(token); throw cause; }
      settled();
      return JSON.stringify({ token, pending: true, state: state(el, req.mediaId) });
    } else if (req.action === 'media.play') {
      if (store.pending.size >= 8) return error('Too many pending media operations; read the media state before trying again.');
      const token = crypto.randomUUID(), pending = { el, id: req.mediaId, done: false, error: null };
      store.pending.set(token, pending);
      Promise.resolve(el.play()).then(() => { pending.done = true; }, (cause) => {
        pending.error = `Media playback failed (${clip(cause?.name, 80)}); check the page playback control and source before trying again.`;
      });
      setTimeout(() => forget(token), 10000);
      return JSON.stringify({ token, pending: true, state: state(el, req.mediaId) });
    } else return error('Unknown media operation; use the advertised media actions.');
    return JSON.stringify({ state: state(el, req.mediaId) });
  } catch {
    return error('The page refused the media operation; read media state and check the page controls before trying again.');
  }
})()
