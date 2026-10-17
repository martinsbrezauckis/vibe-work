/** Isolates the escaped manifest label in full notices and shortened toast previews. Called by bell/toasts. */
export function PluginNoticeText({ text, name }: { text: string; name: string }) {
  const start = text.indexOf("“") + 1;
  if (start === 0) return <>{text}</>;
  const end = Math.min(start + name.length, text.length);
  return (
    <>
      {text.slice(0, start)}
      <bdi>{text.slice(start, end)}</bdi>
      {text.slice(end)}
    </>
  );
}
