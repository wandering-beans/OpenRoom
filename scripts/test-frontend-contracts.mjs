import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';

const calls = [];
const listeners = new Map();
let failure;
const response = { completed: 1, total: 1, cancelled: false, errors: [], folder: '/test-exports' };
globalThis.window = { __TAURI__: {
  core: {
    async invoke(name, args) {
      calls.push({ name, args });
      if (failure) throw failure;
      return name === 'get_language' ? 'en' : response;
    },
    convertFileSrc: path => `asset:${path}`,
  },
  event: {
    async listen(name, handler) { listeners.set(name, handler); return () => listeners.delete(name); },
  },
} };

const { command, invoke, listen, convertFileSrc } = await import('../desktop/ui/backend.ts');
const request = { ids: [1], folder: '/test-exports', naming: '{name}-edited', format: 'jpeg',
  options: { quality: 85, long_edge: 2048, max_bytes: 512000 } };
assert.equal(await command('export_develop_batch', { request }), response);
assert.deepEqual(calls.pop(), { name: 'export_develop_batch', args: { request } }, 'preserve the Rust wire format');
await command('cancel_batch_export');
assert.deepEqual(calls.pop(), { name: 'cancel_batch_export', args: undefined });
await invoke('preview', { id: 1, large: true, bound: 1600 });
assert.deepEqual(calls.pop(), { name: 'preview', args: { id: 1, large: true, bound: 1600 } });
failure = new Error('backend rejected request');
await assert.rejects(command('choose_export_folder'), error => error === failure, 'do not hide backend failures');
failure = undefined;
let progress;
const stop = await listen('develop-export-progress', event => { progress = event.payload; });
listeners.get('develop-export-progress')({ payload: response });
assert.equal(progress, response);
stop();
assert.equal(listeners.size, 0);
assert.equal(convertFileSrc('/test-cache/preview.jpg'), 'asset:/test-cache/preview.jpg');

const { t, errorText, metadataValue } = await import('../desktop/ui/i18n.ts');
const { english } = await import('../desktop/ui/translations.ts');
const parameters = text => [...text.matchAll(/\{(\d+)\}/g)].map(match => match[1]).sort().join(',');
assert.deepEqual(Object.entries(english).filter(([ja, en]) => parameters(ja) !== parameters(en)), []);
assert.equal(t('プロファイル：{0}', '写真・保存済み.jpg'), 'Profile: 写真・保存済み.jpg');
assert.equal(errorText('現像用の作業領域が不足しています。'), 'Insufficient development workspace.');
assert.equal(errorText('/写真/保存済み.jpg'), '/写真/保存済み.jpg');
assert.equal(t('constructor'), 'constructor');
assert.equal(metadataValue('シャッター速度', '2 秒'), '2 s');

const ui = new URL('../desktop/ui/', import.meta.url);
for (const name of await readdir(ui, { recursive: true })) {
  if (!/\.(tsx?|js)$/.test(name) || name === 'backend.ts') continue;
  const source = await readFile(new URL(name, ui), 'utf8');
  assert.ok(!source.includes('__TAURI__'), `${name}: native access must go through backend.ts`);
  assert.ok(!/from\s+['"](?:node:|.*\/desktop\/src\/)/.test(source), `${name}: frontend must not import backend code`);
}
const config = JSON.parse(await readFile(new URL('../desktop/tauri.conf.json', import.meta.url), 'utf8'));
assert.equal(config.build.frontendDist, 'ui-dist');
console.log('Frontend contract checks passed: command payloads, errors, event cleanup, translations, and native boundary.');
