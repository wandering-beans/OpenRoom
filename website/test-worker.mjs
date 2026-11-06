import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {readFile, readdir} from 'node:fs/promises';
import {createWorker} from './worker.mjs';

const db = new DatabaseSync(':memory:');
for (const file of (await readdir(new URL('./drizzle/', import.meta.url))).filter(file => file.endsWith('.sql')).sort()) {
  db.exec(await readFile(new URL(`./drizzle/${file}`, import.meta.url), 'utf8'));
}
const env = {DB: {prepare: sql => ({bind: (...values) => ({run: async () => db.prepare(sql).run(...values)})})}};
const html = '<h1>OpenRoom</h1>';
const worker = createWorker({'/': {body: Buffer.from(html).toString('base64'), type: 'text/html', etag: '"test"'}});
const origin = 'https://deepdive.boats';
const call = (path, method = 'POST', headers = {origin}, bindings = env) =>
  worker.fetch(new Request(origin + path, {method, headers}), bindings);
const count = platform => db.prepare('SELECT clicks FROM download_counts WHERE platform = ?').get(platform)?.clicks ?? 0;
assert.equal((await call('/api/download-click/windows')).status, 204);
assert.equal(count('windows'), 1);
await Promise.all(Array.from({length: 30}, () => call('/api/download-click/windows')));
assert.equal(count('windows'), 31, 'Concurrent requests must not lose increments');
assert.equal((await call('/api/download-click/macos')).status, 204);
assert.equal(count('macos'), 1, 'Platform totals must be independent');
assert.equal((await call('/api/download-click/linux')).status, 404);
assert.equal((await call('/api/download-click/windows/extra')).status, 404);
assert.equal((await call('/api/download-click/windows', 'GET')).status, 405);
assert.equal((await call('/api/download-click/windows', 'POST', {})).status, 403);
assert.equal((await call('/api/download-click/windows', 'POST', {origin: 'https://other.example'})).status, 403);
assert.equal((await call('/api/download-click/windows', 'POST', {origin, 'sec-fetch-site': 'cross-site'})).status, 403);
assert.equal(count('windows'), 31, 'Rejected requests must not count');
assert.equal((await call('/api/download-click/windows', 'POST', {origin}, {})).status, 503);
assert.equal((await call('/api/download-click/windows', 'POST', {origin}, {DB: {prepare() {throw new Error('Quota exhausted');}}})).status, 503);
assert.throws(() => db.prepare('INSERT INTO download_counts VALUES (?, ?)').run('other', 1));
assert.throws(() => db.prepare('UPDATE download_counts SET clicks = -1').run());
assert.equal(await (await call('/', 'GET')).text(), html);
assert.equal(await (await call('/', 'HEAD')).text(), '');
assert.equal((await call('/', 'GET', {'if-none-match': '"test"'})).status, 304);
assert.equal((await call('/', 'POST')).status, 405);
for (const path of ['/api/download-counts', '/db/schema.ts', '/drizzle/0000.sql', '/worker.mjs', '/__proto__']) {
  assert.equal((await call(path, 'GET')).status, 404, 'Counts and server source must not be public');
}
db.close();
const {default: built} = await import('./dist/server/index.js');
for (const route of ['/', '/index.html', '/ja', '/ja/', '/ja/index.html']) {
  const response = await built.fetch(new Request(origin + route), {});
  assert.equal(response.status, 200);
  assert.equal(await response.text(), await readFile(new URL('./index.html', import.meta.url), 'utf8'));
}
for (const file of ['styles.css', 'script.js', 'downloads.mjs', 'releases.mjs', 'assets/icon.png',
  'assets/openroom-develop.jpg', 'assets/openroom-library.jpg', 'assets/openroom-detail.jpg']) {
  const response = await built.fetch(new Request(origin + '/' + file), {});
  assert.equal(response.status, 200);
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), await readFile(new URL(file, import.meta.url)), 'Bundled assets must be unchanged');
}
console.log('Worker passed: real SQLite migration, atomic platform counters, validation, private counts, failure handling, asset responses.');
