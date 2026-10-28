// Visible link labels for annotating native Linux PDFs whose print backend omits links.
// Depends on common document authorization. Called by the one-file writer; never changes the live DOM.
(() => {
  __VW_COMMON__
  const links = Array.from(document.querySelectorAll('a[href]')).slice(0, 500).flatMap((el) => {
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') return [];
    try {
      const url = new URL(el.href, document.baseURI);
      const text = String(el.innerText || '').slice(0, 1000);
      return text && /^(https?|mailto|tel):$/.test(url.protocol) ? [{ url: url.href.slice(0, 2000), text }] : [];
    } catch { return []; }
  });
  return JSON.stringify({ url: String(location.href).slice(0, 2000), links });
})()
