/** Protects ask cards from window-activation clicks. Uses capture listeners; installed by shell wiring and removed on unmount. */
export function guardActivationClick(): () => void {
  let activating = false;
  let blocked = false;
  const blur = () => {
    activating = true;
  };
  const cancel = (event: Event) => {
    event.preventDefault();
    event.stopImmediatePropagation();
  };
  const down = (event: Event) => {
    blocked =
      activating && event.target instanceof Element && event.target.closest(".approval") !== null;
    activating = false;
    if (blocked) cancel(event);
  };
  const finish = (event: Event) => {
    if (blocked) cancel(event);
  };
  const key = () => {
    activating = false;
    blocked = false;
  };
  window.addEventListener("blur", blur);
  window.addEventListener("pointerdown", down, true);
  window.addEventListener("mousedown", finish, true);
  window.addEventListener("mouseup", finish, true);
  window.addEventListener("click", finish, true);
  window.addEventListener("contextmenu", finish, true);
  window.addEventListener("keydown", key, true);
  return () => {
    window.removeEventListener("blur", blur);
    window.removeEventListener("pointerdown", down, true);
    window.removeEventListener("mousedown", finish, true);
    window.removeEventListener("mouseup", finish, true);
    window.removeEventListener("click", finish, true);
    window.removeEventListener("contextmenu", finish, true);
    window.removeEventListener("keydown", key, true);
  };
}
