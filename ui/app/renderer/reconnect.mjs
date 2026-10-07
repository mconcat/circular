export function onNextAction(root, retry) {
  if (!root?.addEventListener) return () => {};
  let armed = true;
  const disarm = () => {
    if (!armed) return;
    armed = false;
    root.removeEventListener('pointerdown', fire, true);
    root.removeEventListener('keydown', fire, true);
  };
  function fire() {
    if (!armed) return;
    disarm();
    retry();
  }
  root.addEventListener('pointerdown', fire, true);
  root.addEventListener('keydown', fire, true);
  return disarm;
}
