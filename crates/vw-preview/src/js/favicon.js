// Bounded favicon pixels for native Browser tabs. The host polls this expression without awaiting a Promise.
// Depends on the admitted page environment; image requests remain inside its native network policy.
(() => {
  const env = window[__VW_FAVICON_NAME__];
  if (!env) return null;
  const url = location.href;
  const link = document.querySelector('link[rel~="icon"]');
  const source = link?.href || new URL("/favicon.ico", url).href;
  let state = env.tabIcon;
  if (!state || state.url !== url || state.source !== source) {
    state = { url, source, done: false, data: null };
    env.tabIcon = state;
    if (!/^https?:|^data:image\//i.test(source) || source.length > 16384) {
      state.done = true;
      return state;
    }
    const image = new Image();
    // Cross-origin icons without CORS safely fall back to the site's letter; credentials are not requested.
    image.crossOrigin = "anonymous";
    image.referrerPolicy = "no-referrer";
    image.onload = () => {
      try {
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 32;
        canvas.getContext("2d").drawImage(image, 0, 0, 32, 32);
        state.data = canvas.toDataURL("image/png");
      } catch (_) { /* Unreadable pixels use the UI's generic icon. */ }
      state.done = true;
    };
    image.onerror = () => { state.done = true; };
    image.src = source;
  }
  return { url: state.url, done: state.done, data: state.data };
})();
