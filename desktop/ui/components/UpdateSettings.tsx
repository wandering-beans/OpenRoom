import { useEffect, useRef, useState } from 'react';
import { command, listen, type AvailableUpdate, type Unlisten } from '../backend';
import { errorText, t } from '../i18n';

export function UpdateSettings({ prepareUpdate, setUpdating, beginAutoUpdate }: {
  prepareUpdate(): Promise<void>;
  setUpdating(value: boolean): void;
  beginAutoUpdate(): boolean;
}) {
  const [version, setVersion] = useState('…');
  const [available, setAvailable] = useState<AvailableUpdate | null>(null);
  const [status, setStatus] = useState('');
  const [checking, setChecking] = useState(false);
  const [installed, setInstalled] = useState(false);
  const [autoUpdate, setAutoUpdate] = useState(true);
  const [preferenceReady, setPreferenceReady] = useState(false);
  const [savingPreference, setSavingPreference] = useState(false);
  const [preferenceStatus, setPreferenceStatus] = useState('');
  const [pendingAuto, setPendingAuto] = useState(false);
  const autoUpdateEnabled = useRef(false);
  const busy = useRef(false);

  useEffect(() => {
    let disposed = false;
    let unlisten: Unlisten | undefined;
    command('get_auto_update').then(async enabled => {
      if (disposed) return;
      autoUpdateEnabled.current = enabled;
      setAutoUpdate(enabled); setPreferenceReady(true);
      if (await command('take_startup_update_check') && !disposed && autoUpdateEnabled.current) await check(true);
    }).catch(failure => {
      if (!disposed) {
        autoUpdateEnabled.current = false; setAutoUpdate(false); setPreferenceReady(true);
        setPreferenceStatus(errorText(failure));
      }
    });
    command('app_version').then(value => { if (!disposed) setVersion(value); })
      .catch(failure => { if (!disposed) setStatus(errorText(failure)); });
    listen<{ downloaded: number; total: number | null }>('update-progress', ({ payload }) => {
      setStatus(payload.total
        ? t('ダウンロード中：{0}%', Math.min(100, Math.floor(payload.downloaded / payload.total * 100)))
        : t('ダウンロード中：{0} MB', (payload.downloaded / 1024 / 1024).toFixed(1)));
    }).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(failure => {
      if (!disposed) setStatus(errorText(failure));
    });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  useEffect(() => {
    if (!pendingAuto) return;
    const attempt = () => {
      if (!autoUpdateEnabled.current || busy.current || !beginAutoUpdate()) return;
      setPendingAuto(false);
      void install();
    };
    const timer = window.setInterval(attempt, 1000);
    attempt();
    return () => window.clearInterval(timer);
  }, [pendingAuto, beginAutoUpdate]);

  async function savePreference(enabled: boolean) {
    const previous = autoUpdateEnabled.current;
    autoUpdateEnabled.current = enabled; setAutoUpdate(enabled);
    if (!enabled) setPendingAuto(false);
    setSavingPreference(true); setPreferenceStatus('');
    try { await command('set_auto_update', { enabled }); }
    catch (failure) {
      autoUpdateEnabled.current = previous; setAutoUpdate(previous);
      setPreferenceStatus(t('設定を保存できませんでした：{0}', errorText(failure)));
    } finally { setSavingPreference(false); }
  }

  async function check(startup = false) {
    if (busy.current || installed) return;
    busy.current = true;
    setPendingAuto(false);
    setChecking(true); setAvailable(null); setStatus(t('更新を確認しています…'));
    try {
      const update = await command('check_app_update');
      setAvailable(update);
      setStatus(update ? t('バージョン {0} に更新できます。', update.version) : t('最新のバージョンです。'));
      if (startup && update && autoUpdateEnabled.current) setPendingAuto(true);
    } catch (failure) { setStatus(t('更新を確認できませんでした：{0}', errorText(failure))); }
    finally { busy.current = false; setChecking(false); }
  }

  async function install() {
    if (busy.current) return;
    busy.current = true; setPendingAuto(false);
    const shell = document.getElementById('app-shell');
    const wasInert = shell?.inert ?? false;
    setUpdating(true);
    if (shell) shell.inert = true;
    try {
      setStatus(t('編集を保存しています…'));
      await prepareUpdate();
      if (!installed) {
        setStatus(t('更新をダウンロードしています…'));
        await command('install_app_update');
        setInstalled(true);
      }
      setStatus(t('更新を適用しました。再起動しています…'));
      await command('restart_catalog');
    } catch (failure) { setStatus(t('更新を完了できませんでした：{0}', errorText(failure))); }
    finally {
      if (shell) shell.inert = wasInert;
      busy.current = false; setUpdating(false);
    }
  }

  return <section aria-labelledby="update-title">
    <h3 id="update-title">{t('バージョンと更新')}</h3>
    <p id="app-version">OpenRoom {version}</p>
    <p className="panel-note">{t('更新を適用すると、編集を保存してアプリを再起動します。')}</p>
    <label className="mcp-enable"><input id="auto-update" type="checkbox" checked={autoUpdate}
      disabled={!preferenceReady || savingPreference || installed}
      onChange={event => { void savePreference(event.target.checked); }} />{t('起動時に更新を確認')}</label>
    <p className="panel-note">{t('オンの場合、起動時に更新が見つかると自動で適用し、再起動します。初期設定はオンです。')}</p>
    <p id="auto-update-status" role="status">{preferenceStatus}</p>
    {pendingAuto && <p id="auto-update-waiting" role="status">{t('処理や編集が終わると自動更新します。')}</p>}
    <div className="mcp-actions">
      <button id="check-update" disabled={checking || installed} onClick={() => { void check(); }}>{t('更新を確認')}</button>
      {(available || installed) && <button id="install-update" disabled={checking} onClick={install}>
        {installed ? t('再起動') : t('更新して再起動')}
      </button>}
    </div>
    <p id="update-status" role="status">{status}</p>
    {available?.notes && <details id="update-notes"><summary>{t('変更内容')}</summary><p>{available.notes}</p></details>}
  </section>;
}
