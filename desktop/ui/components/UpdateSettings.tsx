import { useEffect, useState } from 'react';
import { command, listen, type AvailableUpdate, type Unlisten } from '../backend';
import { errorText, t } from '../i18n';

export function UpdateSettings({ prepareUpdate, setUpdating }: {
  prepareUpdate(): Promise<void>;
  setUpdating(value: boolean): void;
}) {
  const [version, setVersion] = useState('…');
  const [available, setAvailable] = useState<AvailableUpdate | null>(null);
  const [status, setStatus] = useState('');
  const [checking, setChecking] = useState(false);
  const [installed, setInstalled] = useState(false);

  useEffect(() => {
    let disposed = false;
    let unlisten: Unlisten | undefined;
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

  async function check() {
    setChecking(true); setAvailable(null); setStatus(t('更新を確認しています…'));
    try {
      const update = await command('check_app_update');
      setAvailable(update);
      setStatus(update ? t('バージョン {0} に更新できます。', update.version) : t('最新のバージョンです。'));
    } catch (failure) { setStatus(t('更新を確認できませんでした：{0}', errorText(failure))); }
    finally { setChecking(false); }
  }

  async function install() {
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
      setUpdating(false);
    }
  }

  return <section aria-labelledby="update-title">
    <h3 id="update-title">{t('バージョンと更新')}</h3>
    <p id="app-version">OpenRoom {version}</p>
    <p className="panel-note">{t('更新を適用すると、編集を保存してアプリを再起動します。')}</p>
    <div className="mcp-actions">
      <button id="check-update" disabled={checking || installed} onClick={check}>{t('更新を確認')}</button>
      {(available || installed) && <button id="install-update" disabled={checking} onClick={install}>
        {installed ? t('再起動') : t('更新して再起動')}
      </button>}
    </div>
    <p id="update-status" role="status">{status}</p>
    {available?.notes && <details id="update-notes"><summary>{t('変更内容')}</summary><p>{available.notes}</p></details>}
  </section>;
}
