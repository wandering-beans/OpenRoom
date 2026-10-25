import { element as $ } from './dom';

const storageKey = (id: string) => `openroom.sidebar.${id}`;
function read(id: string) {
  try { return localStorage.getItem(storageKey(id)); } catch { return null; }
}
function write(id: string, value: unknown) {
  try { localStorage.setItem(storageKey(id), String(value)); } catch { /* Layout still works when storage is unavailable. */ }
}

export function initSidebar(resized: () => void) {
  for (const id of ['navigator-panel', 'folder-panel']) {
    const panel = $<HTMLDetailsElement>(id);
    panel.open = read(id) !== 'false';
    panel.addEventListener('toggle', () => { write(id, panel.open); resized(); });
  }
  for (const [handleId, targetId, dimension, minimum, limit] of [
    ['sidebar-resize', 'sidebar', 'width', 190, () => Math.min(420, innerWidth * .35)],
    ['navigator-resize', 'navigator-preview', 'height', 80, () => innerHeight * .5],
    ['folder-resize', 'folder-content', 'height', 100, () => innerHeight * .6],
  ] as [string, string, 'width' | 'height', number, () => number][]) {
    const handle = $(handleId), target = $(targetId);
    const horizontal = dimension === 'width';
    let preferred = Number(read(handleId)) || null;
    let drag: { coordinate: number; size: number } | null = null;
    function apply(value: number) {
      const maximum = Math.max(minimum, limit());
      const size = Math.max(minimum, Math.min(maximum, value));
      target.style[dimension] = `${size}px`;
      handle.setAttribute('aria-valuemin', String(minimum));
      handle.setAttribute('aria-valuemax', String(Math.round(maximum)));
      handle.setAttribute('aria-valuenow', String(Math.round(size)));
      resized();
    }
    handle.onpointerdown = event => {
      if (event.button !== 0) return;
      event.preventDefault();
      handle.setPointerCapture(event.pointerId);
      handle.focus();
      drag = { coordinate: horizontal ? event.clientX : event.clientY, size: target.getBoundingClientRect()[dimension] };
    };
    handle.onpointermove = event => {
      if (!drag) return;
      preferred = drag.size + (horizontal ? event.clientX : event.clientY) - drag.coordinate;
      apply(preferred);
    };
    handle.onpointerup = handle.onpointercancel = event => {
      if (!drag) return;
      drag = null;
      if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
      preferred = target.getBoundingClientRect()[dimension];
      write(handleId, preferred);
    };
    handle.onkeydown = event => {
      const decrease = horizontal ? 'ArrowLeft' : 'ArrowUp';
      const increase = horizontal ? 'ArrowRight' : 'ArrowDown';
      if (![decrease, increase, 'Home', 'End'].includes(event.key)) return;
      event.preventDefault(); event.stopPropagation();
      preferred = event.key === 'Home' ? minimum : event.key === 'End' ? limit()
        : target.getBoundingClientRect()[dimension] + (event.key === increase ? 10 : -10);
      apply(preferred);
      write(handleId, target.getBoundingClientRect()[dimension]);
    };
    if (preferred) apply(preferred);
    else {
      handle.setAttribute('aria-valuemin', String(minimum));
      handle.setAttribute('aria-valuemax', String(Math.round(Math.max(minimum, limit()))));
      handle.setAttribute('aria-valuenow', String(Math.round(target.getBoundingClientRect()[dimension])));
    }
    window.addEventListener('resize', () => { if (preferred) apply(preferred); });
  }
}
