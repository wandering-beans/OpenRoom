import {fetchDownloads, RELEASES_URL} from './releases.mjs';
import {setupDownloadForm} from './downloads.mjs';

const form = setupDownloadForm(document.querySelector('#download-form'));
let checking = false;
async function refreshDownloads() {
  if (checking) return;
  checking = true;
  const status = document.querySelector('#release-status');
  try {
    const downloads = await fetchDownloads();
    for (const platform of ['windows', 'macos']) {
      const download = downloads[platform];
      const button = document.querySelector(`[data-platform="${platform}"]`);
      const meta = document.querySelector(`[data-meta="${platform}"]`);
      const notes = document.querySelector(`[data-notes="${platform}"]`);
      form.setDestination(platform, download?.asset.browser_download_url || RELEASES_URL);
      button.textContent = download ? `${platform === 'windows' ? 'Windows' : 'Mac'}版をダウンロード ↓` : 'GitHubで公開版を確認 ↗';
      const size = download && Number.isFinite(download.asset.size) ? ` · ${(download.asset.size / 1048576).toFixed(1)} MB` : '';
      meta.textContent = download ? `${download.tag} · ${download.prerelease ? 'プレリリース' : '公開版'}${size}` : '対応する配布ファイルはGitHubでご確認ください。';
      notes.href = download?.notes || RELEASES_URL;
    }
    status.textContent = 'GitHubの公開リリースからOS別の最新版を表示しています。';
  } catch {
    for (const platform of ['windows', 'macos']) {
      form.setDestination(platform, RELEASES_URL);
      document.querySelector(`[data-platform="${platform}"]`).textContent = 'GitHubで公開版を確認 ↗';
      document.querySelector(`[data-meta="${platform}"]`).textContent = '最新情報を取得できませんでした。';
      document.querySelector(`[data-notes="${platform}"]`).href = RELEASES_URL;
    }
    status.textContent = 'GitHubのリリース一覧で最新版をご確認ください。';
  } finally { checking = false; }
}
refreshDownloads();
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible') refreshDownloads();
});
