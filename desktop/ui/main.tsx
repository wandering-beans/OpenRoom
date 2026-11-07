import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { StartupScreen } from './components/StartupScreen';
import { Workspace } from './components/Workspace';
import { TermsScreen } from './components/TermsScreen';
import { command } from './backend';
import { language, translateStaticPage } from './i18n';
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
const root = createRoot(container);
document.documentElement.lang = language;
async function start() {
  if (!await command('start_workspace')) {
    root.render(<TermsScreen onAccept={start} />);
    return;
  }
  flushSync(() => root.render(<App />));
  translateStaticPage();
  await import('./app');
}

try {
  await start();
} catch (failure) {
  root.render(<main role="alert">{String(failure)}</main>);
  console.error(failure);
}
