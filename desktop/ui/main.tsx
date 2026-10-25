import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { StartupScreen } from './components/StartupScreen';
import { Workspace } from './components/Workspace';
import './style.css';
import './react-layout.css';

// The stable shell hosts independent React views; typed controllers coordinate native work and image geometry.
function App() {
  return <>
    <StartupScreen />
    <Workspace />
    <div id="photo-export-root" />
    <div id="preferences-root" />
  </>;
}

const container = document.getElementById('root');
if (!container) throw new Error('Missing application root');
flushSync(() => createRoot(container).render(<App />));

try {
  const { translateStaticPage } = await import('./i18n');
  translateStaticPage();
  await import('./app');
} catch (failure) {
  const status = document.getElementById('startup-loading');
  if (status) {
    status.textContent = String(failure);
    status.setAttribute('role', 'alert');
  }
  console.error(failure);
}
