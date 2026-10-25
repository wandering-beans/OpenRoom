import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';

// Controllers may measure the committed view immediately after an update.
export function createView(container: Element) {
  const root = createRoot(container);
  return {
    render(content: ReactNode) { flushSync(() => root.render(content)); },
    unmount() { flushSync(() => root.unmount()); },
  };
}
