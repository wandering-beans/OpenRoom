import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react';
import { createRoot } from 'react-dom/client';
import { createPortal, flushSync } from 'react-dom';
import { command, listen, type Language, type McpStatus, type Unlisten } from '../backend';
import { errorText, language, t } from '../i18n';

interface PreferencesActions {
  changeLanguage(value: Language): Promise<void>;
  error(failure: unknown): void;
}
interface PreferencesHandle {
  showStatus(status: McpStatus): void;
  showJobStatus(message: string): void;
}

function Preferences({ changeLanguage, error, ref }: PreferencesActions & { ref: Ref<PreferencesHandle> }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [enabled, setEnabled] = useState(false);
  const [port, setPort] = useState('39127');
  const [url, setUrl] = useState('');
  const [status, setStatus] = useState(t('停止中'));
  const [token, setToken] = useState('');
  const [tokenVisible, setTokenVisible] = useState(false);
  const [pending, setPending] = useState(false);
  const [selectedLanguage, setSelectedLanguage] = useState<Language>(language);
  const [languagePending, setLanguagePending] = useState(false);
  const [languageStatus, setLanguageStatus] = useState('');
  const [jobStatus, setJobStatus] = useState('');

  function showStatus(value: McpStatus) {
    setEnabled(value.enabled);
    setPort(String(value.port));
    setUrl(value.url);
    setStatus(value.error || (value.running ? t('接続を受け付けています') : t('停止中')));
  }
  function clearToken() { setToken(''); setTokenVisible(false); }
  useImperativeHandle(ref, () => ({ showStatus, showJobStatus: setJobStatus }));

  useEffect(() => {
    async function open() {
      if (document.getElementById('app-shell')?.inert || document.querySelector('dialog[open]')) return;
      dialog.current?.showModal();
      clearToken();
      try { showStatus(await command('mcp_status')); }
      catch (failure) { setStatus(errorText(failure)); }
    }
    let disposed = false;
    let unlisten: Unlisten | undefined;
    document.addEventListener('open-preferences', open);
    listen('open-preferences', open).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(error);
    return () => {
      disposed = true;
      unlisten?.();
      document.removeEventListener('open-preferences', open);
    };
  }, [error]);

  async function settingsAction(action: () => Promise<McpStatus>) {
    setPending(true);
    try { showStatus(await action()); clearToken(); }
    catch (failure) { setStatus(errorText(failure)); }
    finally { setPending(false); }
  }
  async function showToken() {
    try {
      const value = await command('mcp_token');
      if (dialog.current?.open) { setToken(value); setTokenVisible(true); }
    } catch (failure) { setStatus(errorText(failure)); }
  }
  async function copyText(value: string) {
    try { await navigator.clipboard.writeText(value); }
    catch {
      const field = document.createElement('textarea');
      field.value = value;
      dialog.current!.append(field);
      field.select();
      const copied = document.execCommand('copy');
      field.remove();
      if (!copied) throw Error(t('コピーできません。表示された値を選択してコピーしてください。'));
    }
  }
  async function copyToken() {
    try { await copyText(await command('mcp_token')); setStatus(t('認証トークンをコピーしました')); }
    catch (failure) { setStatus(errorText(failure)); }
  }
  async function copyUrl() {
    try { await copyText(url); setStatus(t('接続URLをコピーしました')); }
    catch (failure) { setStatus(errorText(failure)); }
  }
  async function selectLanguage(value: Language) {
    setSelectedLanguage(value);
    setLanguagePending(true);
    try { await changeLanguage(value); }
    catch (failure) { setLanguageStatus(errorText(failure)); }
    finally { setSelectedLanguage(language); setLanguagePending(false); }
  }

  return <dialog id="preferences-dialog" ref={dialog} aria-labelledby="preferences-title" onClose={clearToken}>
    <h2 id="preferences-title">{t('環境設定')}</h2>
    <label className="language-setting" htmlFor="ui-language">{t('言語 / Language')}<select id="ui-language" value={selectedLanguage}
      disabled={languagePending} onChange={event => { void selectLanguage(event.target.value as Language); }}>
      <option value="ja">日本語</option><option value="en">English</option>
    </select></label>
    <p className="panel-note">{t('言語を変更すると、編集を保存して画面を再読み込みします。')}</p>
    <p id="language-status" role="status">{languageStatus}</p>
    <h3>{t('AI連携')}</h3>
    <p>{t('同じPCのAIエージェントから、開いているカタログを操作できます。')}</p>
    <p id="mcp-status" role="status">{status}</p>
    <fieldset id="mcp-settings" disabled={pending}>
      <label className="mcp-enable"><input id="mcp-enabled" type="checkbox" checked={enabled} onChange={event => setEnabled(event.target.checked)} />{t('MCPサーバーを有効にする')}</label>
      <p className="panel-note">{t('有効状態を保存し、次回起動時にも接続を受け付けます。接続後の編集・書き出しはそのまま実行されます。')}</p>
      <label>{t('ポート')}<input id="mcp-port" type="number" min="1" max="65535" value={port} onChange={event => setPort(event.target.value)} /></label>
      <button id="mcp-apply" onClick={() => settingsAction(() => command('mcp_configure', { enabled, port: Number(port) }))}>{t('設定を適用')}</button>
      <label>{t('接続URL')}<input id="mcp-url" value={url} readOnly /></label><button id="mcp-copy-url" onClick={copyUrl}>{t('URLをコピー')}</button>
      <label>{t('認証トークン')}<input id="mcp-token" type={tokenVisible ? 'text' : 'password'} value={token} readOnly autoComplete="off" /></label>
      <div className="mcp-actions"><button id="mcp-show-token" onClick={showToken}>{t('表示')}</button><button id="mcp-copy-token" onClick={copyToken}>{t('トークンをコピー')}</button><button id="mcp-rotate" onClick={() => settingsAction(() => command('mcp_rotate'))}>{t('再発行')}</button></div>
      <p className="panel-note">{t('接続方式はStreamable HTTPです。Authorizationヘッダーに「Bearer 」とトークンを指定してください。再発行すると以前のトークンは使えなくなります。')}</p>
    </fieldset>
    <p id="mcp-job-status" role="status">{jobStatus}</p>
    <button id="mcp-close" onClick={() => dialog.current?.close()}>{t('閉じる')}</button>
  </dialog>;
}

export function createPreferences(actions: PreferencesActions): PreferencesHandle {
  const container = document.getElementById('preferences-root');
  if (!container) throw new Error('Missing preferences root');
  const handle = { current: null as PreferencesHandle | null };
  flushSync(() => createRoot(container).render(createPortal(<Preferences {...actions} ref={handle} />, document.body)));
  return {
    showStatus: status => handle.current!.showStatus(status),
    showJobStatus: message => handle.current!.showJobStatus(message),
  };
}
