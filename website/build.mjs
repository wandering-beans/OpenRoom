import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';

const root = new URL('./', import.meta.url);
const manifest = JSON.parse(await readFile(new URL('.openai/hosting.json', root), 'utf8'));
if (manifest.static || manifest.d1 !== 'DB') throw new Error('The counter requires a Worker with a DB binding.');
const assets = {};
const files = {
  'index.html': 'text/html; charset=utf-8',
  'styles.css': 'text/css; charset=utf-8',
  'script.js': 'text/javascript; charset=utf-8',
  'downloads.mjs': 'text/javascript; charset=utf-8',
  'releases.mjs': 'text/javascript; charset=utf-8',
  'assets/icon.png': 'image/png',
  'assets/openroom-develop.jpg': 'image/jpeg',
  'assets/openroom-library.jpg': 'image/jpeg',
  'assets/openroom-detail.jpg': 'image/jpeg'
};
for (const [file, type] of Object.entries(files)) {
  const bytes = await readFile(new URL(file, root));
  assets[`/${file}`] = {type, body: bytes.toString('base64'), etag: `"${createHash('sha256').update(bytes).digest('hex')}"`};
}
for (const path of ['/', '/ja', '/ja/', '/ja/index.html']) assets[path] = assets['/index.html'];
const worker = await readFile(new URL('worker.mjs', root), 'utf8');
await mkdir(new URL('dist/server/', root), {recursive: true});
await mkdir(new URL('dist/.openai/', root), {recursive: true});
await writeFile(new URL('dist/server/index.js', root), `${worker}\nexport default createWorker(${JSON.stringify(assets)});\n`);
await writeFile(new URL('dist/.openai/hosting.json', root), JSON.stringify(manifest, null, 2) + '\n');
console.log('Built the Worker with the existing website assets and download counter.');
