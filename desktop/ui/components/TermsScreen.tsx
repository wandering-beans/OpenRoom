import { useState } from 'react';
import { command } from '../backend';
import { language } from '../i18n';
import terms from '../../../TERMS.md?raw';
import disclaimer from '../../../DISCLAIMER.md?raw';
import license from '../../../LICENSE?raw';

export function TermsScreen({ onAccept }: { onAccept: () => Promise<void> }) {
  const en = language === 'en';
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  async function choose(accept: boolean) {
    setBusy(true);
    setError('');
    try {
      if (accept) {
        await command('accept_terms');
        await onAccept();
      } else {
        await command('close_window');
      }
    } catch (failure) {
      setError(String(failure));
      setBusy(false);
    }
  }
  return <main id="terms-screen" aria-labelledby="terms-title">
    <h1 id="terms-title">{en ? 'OpenRoom Terms of Use' : 'OpenRoom 利用規約'}</h1>
    <p>{en ? 'Please review the terms and warranty disclaimer before opening your catalog. The full terms below are in Japanese.' : '利用規約と無保証・免責の内容をご確認ください。'}</p>
    <section className="terms-documents" tabIndex={0} aria-label={en ? 'Terms and disclaimer' : '利用規約と免責事項'}>
      <pre lang="ja">{terms}</pre><pre lang="ja">{disclaimer}</pre>
      <details><summary>{en ? 'Full GNU GPL license' : 'GPLライセンス全文（英語）'}</summary><pre lang="en">{license}</pre></details>
    </section>
    {error && <p role="alert">{error}</p>}
    <footer>
      <button id="terms-decline" disabled={busy} onClick={() => void choose(false)}>{en ? 'Decline and quit' : '同意せず終了'}</button>
      <button id="terms-accept" disabled={busy} onClick={() => void choose(true)}>{en ? 'Agree and continue' : '同意して続ける'}</button>
    </footer>
  </main>;
}
