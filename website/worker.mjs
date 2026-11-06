// Counts are private to Sites' authenticated database viewer. There is no public read endpoint.
export function createWorker(assets) {
  return {
    async fetch(request, env) {
      const url = new URL(request.url);
      const counterPrefix = '/api/download-click/';
      const headers = {'cache-control': 'no-store', 'x-content-type-options': 'nosniff'};
      if (url.pathname.startsWith(counterPrefix)) {
        const platform = url.pathname.slice(counterPrefix.length);
        if (!['windows', 'macos'].includes(platform)) return new Response(null, {status: 404, headers});
        if (request.method !== 'POST') return new Response(null, {status: 405, headers: {...headers, allow: 'POST'}});
        if (request.headers.get('origin') !== url.origin ||
            (request.headers.has('sec-fetch-site') && request.headers.get('sec-fetch-site') !== 'same-origin')) {
          return new Response(null, {status: 403, headers});
        }
        try {
          // Atomic increment avoids lost clicks when several visitors click together.
          await env.DB.prepare(`INSERT INTO download_counts (platform, clicks) VALUES (?, 1)
            ON CONFLICT(platform) DO UPDATE SET clicks = download_counts.clicks + 1`).bind(platform).run();
          return new Response(null, {status: 204, headers});
        } catch {
          return new Response(null, {status: 503, headers});
        }
      }
      const asset = assets[url.pathname];
      if (!asset || !Object.hasOwn(assets, url.pathname)) return new Response('Not found', {status: 404, headers});
      if (!['GET', 'HEAD'].includes(request.method)) return new Response(null, {status: 405, headers: {...headers, allow: 'GET, HEAD'}});
      const assetHeaders = {
        'content-type': asset.type, 'cache-control': 'public, max-age=0, must-revalidate',
        etag: asset.etag, 'x-content-type-options': 'nosniff'
      };
      if (request.headers.get('if-none-match') === asset.etag) return new Response(null, {status: 304, headers: assetHeaders});
      const body = request.method === 'HEAD' ? null : Uint8Array.from(atob(asset.body), char => char.charCodeAt(0));
      return new Response(body, {headers: assetHeaders});
    }
  };
}
