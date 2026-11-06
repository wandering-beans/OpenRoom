export const RELEASES_URL = 'https://github.com/wandering-beans/OpenRoom/releases';
export const API_URL = 'https://api.github.com/repos/wandering-beans/OpenRoom/releases?per_page=100';

function trustedUrl(value, prefix) {
  try {
    const url = new URL(value);
    return url.origin === 'https://github.com' && url.pathname.startsWith(prefix) && !url.username && !url.password;
  } catch { return false; }
}

export function selectDownloads(releases) {
  if (!Array.isArray(releases)) throw new Error('Invalid releases response');
  const result = {};
  const published = releases.filter(release => release && !release.draft && Number.isFinite(Date.parse(release.published_at)))
    .sort((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at));
  for (const release of published) {
    if (!trustedUrl(release.html_url, '/wandering-beans/OpenRoom/releases/tag/')) continue;
    const assets = Array.isArray(release.assets) ? release.assets : [];
    for (const platform of ['windows', 'macos']) {
      if (result[platform]) continue;
      // Prefer an installer/app ZIP over update archives, signatures, and source bundles.
      const patterns = platform === 'windows'
        ? [/^OpenRoom[-_].*(?:x64|amd64|x86_64).*setup\.exe$/i, /^OpenRoom[-_].*(?:x64|amd64|x86_64).*\.msi$/i, /^OpenRoom[-_].*(?:x64|amd64|x86_64).*\.exe$/i]
        : [/^OpenRoom[-_].*(?:macos|darwin).*(?:arm64|aarch64|universal).*\.dmg$/i, /^OpenRoom[-_].*(?:macos|darwin).*(?:arm64|aarch64|universal).*\.zip$/i];
      const usable = assets.filter(asset => asset && typeof asset.name === 'string' && !/(?:source|dependenc|\.sig$|\.app\.tar\.gz$)/i.test(asset.name)
        && !(platform === 'windows' && /(?:arm64|aarch64|ia32|i686|win32|x86(?!_64))/i.test(asset.name))
        && trustedUrl(asset.browser_download_url, '/wandering-beans/OpenRoom/releases/download/'));
      const asset = patterns.map(pattern => usable.find(item => pattern.test(item.name))).find(Boolean);
      if (asset) result[platform] = {asset, tag: release.tag_name, prerelease: release.prerelease === true, published: release.published_at, notes: release.html_url};
    }
  }
  return result;
}

export async function fetchDownloads(fetcher = fetch) {
  let next = API_URL;
  const releases = [];
  // Follow GitHub pagination so a platform with slower releases remains discoverable.
  while (next) {
    const response = await fetcher(next, {headers: {Accept: 'application/vnd.github+json'}, cache: 'no-cache', signal: AbortSignal.timeout(10000)});
    if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
    const page = await response.json();
    if (!Array.isArray(page)) throw new Error('Invalid releases response');
    releases.push(...page);
    const downloads = selectDownloads(releases);
    if (downloads.windows && downloads.macos) return downloads;
    const link = response.headers.get('link')?.match(/<([^>]+)>; rel="next"/);
    next = link?.[1];
    if (next) {
      const url = new URL(next);
      if (url.origin !== 'https://api.github.com' || url.pathname !== '/repos/wandering-beans/OpenRoom/releases') throw new Error('Invalid pagination URL');
    }
  }
  return selectDownloads(releases);
}
