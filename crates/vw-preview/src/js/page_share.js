// One-use physical menu share choices stay in this isolated document. Addresses remain inert text.
/* exported pageShare */
const pageShare = (() => {
let shareChoice = null;
let shareIntent = false;
let shareExpires = 0;
const takeShareChoice = () => {
  const choice = performance.now() < shareExpires ? shareChoice : null;
  shareChoice = null;
  shareIntent = false;
  return choice;
};
Object.defineProperty(window, "__vw_share_intent", {
  value: Object.freeze({ take: () => {
    const ready = shareIntent && performance.now() < shareExpires;
    shareIntent = false;
    return ready;
  } }), enumerable: false, configurable: false, writable: false,
});
const addShareItems = (menu, button, event, valid, close) => {
  const path = event.composedPath();
  const link = path.find((node) => node?.matches?.("a[href],area[href]"));
  const image = path.find((node) => node?.matches?.("img"));
  const source = { pageUrl: location.href, pageTitle: document.title.slice(0, 4096) };
  const choices = [["page", "Share page to chat", null]];
  if (link?.href) choices.push(["link", "Share link to chat", link.href]);
  if (image?.currentSrc || image?.src) choices.push(["image", "Share image to chat", image.currentSrc || image.src]);
  for (const [kind, label, url] of choices) {
    if (url && url.length > 16384) continue;
    const item = button(label);
    item.addEventListener("click", (click) => {
      click.preventDefault();
      click.stopImmediatePropagation();
      if (click.isTrusted && valid() && location.href === source.pageUrl) {
        shareChoice = { ...source, kind, url };
        shareIntent = true;
        shareExpires = performance.now() + 10000;
      }
      close();
    }, true);
    menu.appendChild(item);
  }
};

return {take:takeShareChoice, append:addShareItems, reset:() => {shareChoice=null;shareIntent=false;}};
})();
