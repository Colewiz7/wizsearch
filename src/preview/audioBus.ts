// One audio preview at a time, app-wide. Tiles register their <audio> element
// here before playing; starting a new one pauses the previous.

let current: HTMLAudioElement | null = null;

export function playExclusive(el: HTMLAudioElement) {
  if (current && current !== el) {
    current.pause();
  }
  current = el;
  void el.play().catch(() => {
    /* user gesture / decode issues; the tile shows its own error state */
  });
}

export function stopAll() {
  current?.pause();
  current = null;
}
