// Run on Windows with Node 24, Playwright and a staged Release desktop executable.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { execFileSync, spawn } = require('node:child_process');
const { DatabaseSync } = require('node:sqlite');
const { chromium } = require('../desktop/node_modules/playwright');

const [executable, cli, ...sources] = process.argv.slice(2);
assert(executable && cli && sources.length, 'Pass release.exe, advanced-explorer.exe, then RAW folder(s)');
const repo = path.resolve(__dirname, '..');
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'openroom-import-bench-'));
const photos = path.join(temp, 'photos');
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const raw = /\.(cr2|cr3|crw|nef|nrw|raf|srw|sr2|srf|arw|orf|rw2|rwl|pef|iiq|fff|3fr|dng)$/i;

function query(db, sql) {
  const catalog = new DatabaseSync(db,{readOnly:true, timeout:15000});
  try { return catalog.prepare(sql).all(); }
  finally { catalog.close(); }
}

function files(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const full = path.join(dir, entry.name);
    return entry.isDirectory() ? files(full) : entry.isFile() ? [full] : [];
  });
}
function sizes(dir) {
  const list = fs.existsSync(dir) ? files(dir) : [];
  return list.reduce((total, file) => total + (fs.statSync(file, {throwIfNoEntry:false})?.size || 0), 0);
}

async function round(index, avif) {
  const root = path.join(temp, String(index)), data = path.join(root, 'data');
  fs.mkdirSync(data, {recursive:true});
  const db = path.join(data, 'catalog.db'), previews = db + '.previews';
  execFileSync(cli, ['--db', db, 'add-root', photos]);
  fs.writeFileSync(path.join(data, 'accepted-terms.json'), JSON.stringify(['TERMS.md','DISCLAIMER.md'].map(f =>
    fs.readFileSync(path.join(repo, f), 'utf8').replace(/\r\n/g, '\n'))));
  fs.writeFileSync(path.join(data, 'auto-update.json'), 'false');
  const freeBytesBeforeLaunch = os.freemem();
  let child, browser, page, sample, monitor;
  let peakBytes = 0, peakRss = 0, progress, samples = 0, skippedSamples = 0;
  async function launch() {
    const listener = require('node:net').createServer();
    await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
    const port = listener.address().port;
    await new Promise(resolve => listener.close(resolve));
    child = spawn(executable, [], {windowsHide:true, stdio:'ignore', env:{ ...process.env,
      AE_DATA_DIR:data, WEBVIEW2_USER_DATA_FOLDER:path.join(root,'webview-'+port),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1` }});
    page = null;
    for (let attempt=0; attempt<80 && !page; attempt++) {
      try {
        browser = await chromium.connectOverCDP('http://127.0.0.1:'+port, {timeout:800});
        page = browser.contexts()[0].pages().find(p => p.url().includes('tauri.localhost'));
        if (!page) await browser.close();
      } catch {}
      if (!page) await pause(200);
    }
    assert(page, 'No WebView2 page');
    await page.waitForFunction(() => document.querySelector('#preferences-dialog') && !document.querySelector('#scan').disabled);
    assert.equal(path.resolve(await page.evaluate(() => __TAURI__.core.invoke('catalog_path'))),db);
  }
  try {
    await launch();
    await page.evaluate(avif => __TAURI__.core.invoke('prepare_library_previews',{avifSupported:avif}), avif);
    await page.exposeFunction('recordBenchProgress', report => { progress = report; });
    await page.evaluate(() => __TAURI__.event.listen('preview-progress', e => window.recordBenchProgress(e.payload)).then(() => true));

    // Sum working sets only for the staged executable (main process and its render workers).
    const cpuSamples = new Map();
    const profileCpu = process.env.AE_BENCH_CPU === '1';
    const processList = "Get-Process advanced-explorer-desktop -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $env:OPENROOM_BENCH_EXE }";
    const measurement = profileCpu
      ? `$p = @(${processList}); $lines = @($p | ForEach-Object { '{0},{1},{2}' -f $_.Id,$_.WorkingSet64,([long]$_.TotalProcessorTime.TotalMilliseconds) }); [Console]::WriteLine(($lines -join ';'))`
      : `$n = (${processList} | Measure-Object WorkingSet64 -Sum).Sum; Write-Output ([long]$n)`;
    monitor = spawn('powershell.exe', ['-NoProfile','-Command',
      `while ($true) { ${measurement}; Start-Sleep -Milliseconds 250 }`],
      {windowsHide:true, env:{...process.env, OPENROOM_BENCH_EXE:path.resolve(executable)}, stdio:['ignore','pipe','ignore']});
    let partial = '';
    monitor.stdout.on('data', bytes => {
      const lines = (partial+String(bytes)).split(/\r?\n/);
      partial = lines.pop();
      for (const line of lines.filter(Boolean)) {
        if (!profileCpu) { peakRss = Math.max(peakRss, Number(line)||0); continue; }
        let rss = 0;
        for (const tuple of line.split(';')) {
          const [pid,working,cpu] = tuple.split(',').map(Number);
          if (!Number.isFinite(cpu)) continue;
          rss += working;
          const previous = cpuSamples.get(pid);
          cpuSamples.set(pid, {first:previous?.first ?? cpu,last:cpu});
        }
        peakRss = Math.max(peakRss,rss);
      }
    });
    sample = setInterval(() => {
      try { peakBytes = Math.max(peakBytes,sizes(previews)); samples++; }
      catch (error) {
        // An import worker can release a private directory between readdir and stat on Windows.
        if (!['ENOENT','ENOTDIR','EPERM','EACCES','EBUSY'].includes(error.code)) throw error;
        skippedSamples++;
      }
    },100);
    await page.locator('#grid-view').click();
    await page.keyboard.press('Control+f');
    await page.locator('#scan').waitFor({state:'visible'});
    await page.evaluate(() => {
      window.benchFirstGrid = null;
      document.querySelector('#scan').addEventListener('click', () => {
        const start = performance.now();
        function poll() {
          const visible = [...document.querySelectorAll('.tile img')].some(i => i.complete && i.naturalWidth && i.checkVisibility());
          if (visible) window.benchFirstGrid = performance.now()-start;
          else requestAnimationFrame(poll);
        }
        requestAnimationFrame(poll);
      }, {capture:true,once:true});
    });
    const started = performance.now();
    await page.locator('#scan').click();
    await page.waitForFunction(() => document.querySelector('#scan').disabled);
    await page.waitForFunction(() => !document.querySelector('#scan').disabled, undefined, {timeout:3600000});
    const ms = performance.now() - started;
    const importEndBytes = sizes(previews);
    clearInterval(sample);
    monitor.kill();
    assert(samples > 0);
    const rows = query(db,'select i.id,i.path,p.data from items i left join library_previews p on i.id=p.item_id');
    const images = rows.map(({data}) => data && JSON.parse(data));
    const codecs = [...new Set(images.map(row=>row?.extension))];
    assert.deepEqual(codecs,[avif ? 'avif' : 'jpg']);
    assert.equal(images.length, fs.readdirSync(photos).length);
    const available = images.filter(i => i?.raw_developed && !i.camera_fallback && i.large_bytes && i.thumbnail_bytes &&
      Math.max(...i.large) === Math.min(1600, Math.max(i.width,i.height)) && Math.max(...i.thumbnail) === Math.min(384,Math.max(...i.large)));
    const output = {};
    for (const folder of ['large','thumbnails']) {
      const list = files(path.join(previews,folder));
      output[folder] = {count:list.length,bytes:sizes(path.join(previews,folder)),
        sha256:list.map(f=>createHash('sha256').update(fs.readFileSync(f)).digest('hex')).sort()};
    }
    const decodedPixels = await page.evaluate(async rows => {
      const hashes = {};
      for (const row of rows) {
        const record = JSON.parse(row.data);
        for (const bound of [384,1600]) {
          const preview = await __TAURI__.core.invoke('preview',{id:row.id,large:bound===1600,bound});
          const image = new Image();
          image.crossOrigin = "anonymous";
          image.src = preview.inline_image || __TAURI__.core.convertFileSrc(preview.path);
          await image.decode();
          const expected = bound === 384 ? record.thumbnail : record.large;
          if (image.naturalWidth !== expected[0] || image.naturalHeight !== expected[1]) throw Error('Preview dimensions changed');
          const canvas = document.createElement('canvas');
          [canvas.width,canvas.height] = expected;
          const context = canvas.getContext('2d');
          context.drawImage(image,0,0);
          const digest = await crypto.subtle.digest('SHA-256',context.getImageData(0,0,...expected).data);
          hashes[row.path+'@'+bound] = Array.from(new Uint8Array(digest),b=>b.toString(16).padStart(2,'0')).join('');
        }
      }
      return hashes;
    },rows);
    const importProgress = progress;
    const firstGridMs = await page.evaluate(() => window.benchFirstGrid);
    const warmAt = performance.now();
    const warm = await page.evaluate(() => __TAURI__.core.invoke('scan',{rootId:null}));
    const observedCpuMs = profileCpu ? [...cpuSamples.values()].reduce((sum,p)=>sum+p.last-p.first,0) : undefined;
    console.log(JSON.stringify({index,avif,codecs,freeBytesBeforeLaunch,observedCpuMs,ms,firstGridMs,importEndBytes,decoded:Object.keys(decodedPixels).length,decodedPixels,available:available.length,total:rows.length,progress:importProgress,
      warmMs:performance.now()-warmAt,cached:warm.previews,samples,skippedSamples,peakBytes,peakAppWorkerRss:peakRss,
      finalBytes:sizes(previews),output,dimensions:rows.map((r,i)=>[path.basename(r.path),images[i]?.width,images[i]?.height])}));
    if (process.env.AE_BENCH_REFERENCE_EXE) {
      const recipe = path.join(root,'neutral.json'), frame = path.join(root,'reference.png');
      fs.writeFileSync(recipe,'{}');
      const quality = [];
      for (const row of rows) {
        execFileSync(process.env.AE_BENCH_REFERENCE_EXE,['--develop-agent-preview',path.join(photos,row.path),frame,recipe,'1600'],
          {windowsHide:true,stdio:'ignore',timeout:180000});
        const reference = 'data:image/png;base64,'+fs.readFileSync(frame).toString('base64');
        const metrics = await page.evaluate(async ({id,reference}) => {
          const preview = await __TAURI__.core.invoke('preview',{id,large:true,bound:1600});
          async function pixels(url) {
            const image = new Image();
            image.crossOrigin = 'anonymous'; image.src = url; await image.decode();
            const canvas = document.createElement('canvas');
            canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
            const context = canvas.getContext('2d');
            context.drawImage(image,0,0);
            return context.getImageData(0,0,canvas.width,canvas.height).data;
          }
          const [original,output] = await Promise.all([pixels(reference),pixels(__TAURI__.core.convertFileSrc(preview.path))]);
          if (original.length !== output.length) throw Error('Reference dimensions differ');
          let squared = 0;
          for (let i=0;i<original.length;i+=4) for (let c=0;c<3;c++) squared += (original[i+c]-output[i+c])**2;
          const samples = original.length*3/4;
          return {squared,samples,psnr:10*Math.log10(255**2*samples/squared)};
        },{id:row.id,reference});
        quality.push({photo:row.path,...metrics});
      }
      fs.unlinkSync(frame);
      console.log(JSON.stringify({quality, avif}));
    }
    function addBatchRoot(label) {
      const folder = fs.mkdtempSync(path.join(root,label));
      for (const name of fs.readdirSync(photos)) fs.copyFileSync(path.join(photos,name), path.join(folder,name));
      execFileSync(cli,['--db',db,'add-root',folder]);
      return query(db,'select max(id) as id from roots')[0].id;
    }
    if (index === 0 && process.env.AE_BENCH_UI) await require('./benchmark-raw-navigation.cjs')(page, previews, addBatchRoot);
    if (index === 0 && process.env.AE_BENCH_INTERACTIONS) {
      const first = rows.find(row => /01\.nef$/i.test(row.path)) || rows[0];
      const second = rows.find(row => /02\.nef$/i.test(row.path)) || rows.find(row => row.id !== first.id);
      await require('./test-native-raw-interactions.cjs')(page, first, second);
    }
    if (index === 0 && process.env.AE_BENCH_RECOVERY) {
      const history = () => {
        const catalog = new DatabaseSync(db,{readOnly:true,timeout:15000});
        try { return ['develop_edits','develop_history'].map(table => catalog.prepare('select * from '+table+' order by rowid').all()); }
        finally { catalog.close(); }
      };
      await page.evaluate(async () => {
        const recipe = await __TAURI__.core.invoke('get_develop',{id:1});
        const settings = {...recipe.settings,exposure:0.75};
        await __TAURI__.core.invoke('save_develop',{id:1,settings,expectedRevision:recipe.revision,
          steps:[{settings,label:'Benchmark exposure'}]});
      });
      const original = history();
      const recoveryRoot = addBatchRoot('recovery-');
      const ids = query(db,'select id from items order by id').map(r=>r.id);
      await page.evaluate(async rootId => {
        await __TAURI__.core.invoke('set_develop_prefetch',{ids:[]});
        let stopped = false;
        const off = await __TAURI__.event.listen('preview-progress', e => {
          if (!stopped && e.payload.ready > 0 && e.payload.ready < e.payload.total) {
            stopped = true;
            void __TAURI__.core.invoke('cancel_scan');
          }
        });
        try {
          const interrupted = await __TAURI__.core.invoke('scan',{rootId});
          if (!interrupted.previews.cancelled) throw Error('Scan did not cancel');
        } finally { off(); }
        window.benchResume = __TAURI__.core.invoke('scan',{rootId}).catch(()=>{});
      },recoveryRoot);
      const working = path.join(previews,'working');
      let active = false;
      for (let i=0;i<300 && !active;i++) {
        active = fs.readdirSync(working).some(name=>name.startsWith('import-'));
        if (!active) await pause(20);
      }
      assert(active && child.exitCode === null, 'Expected a live benchmark import');
      execFileSync('taskkill.exe',['/pid',String(child.pid),'/t','/f'],{windowsHide:true,stdio:'ignore'});
      await browser.close().catch(()=>{});
      await pause(1000);
      await launch();
      // Startup prepares any unfinished photos and prunes abandoned private work directories.
      await page.waitForFunction(() => document.querySelector('#startup-screen').hidden && !document.querySelector('#scan').disabled,
        undefined,{timeout:180000});
      const restored = await page.evaluate(()=>__TAURI__.core.invoke('scan',{rootId:null}));
      assert.equal(restored.previews.failed,0);
      const restoredRows = query(db,'select data from library_previews order by item_id').map(row=>JSON.parse(row.data));
      const count = query(db,'select count(*) as n from items')[0].n;
      assert.equal(restoredRows.length,count);
      assert(restoredRows.every(row=>row.raw_developed && row.large_bytes && row.thumbnail_bytes));
      const allIds = query(db,'select id from items order by id').map(row=>row.id);
      const linked = await page.evaluate(async ids => {
        const paths = [];
        for (const id of ids) for (const bound of [384,1600]) {
          const image = await __TAURI__.core.invoke('preview',{id,large:bound===1600,bound});
          paths.push(image.path);
        }
        return paths;
      },allIds);
      const stored = [...files(path.join(previews,'large')), ...files(path.join(previews,'thumbnails'))];
      assert.deepEqual(new Set(stored.map(f=>fs.realpathSync(f))),new Set(linked.map(f=>fs.realpathSync(f))));
      const folders = fs.readdirSync(working,{withFileTypes:true}).filter(entry=>entry.isDirectory()).map(entry=>entry.name);
      assert(folders.every(name=>name.startsWith('renderer-') || name.startsWith('develop-prefetch-')));
      assert(folders.filter(name=>name.startsWith('develop-prefetch-')).length <= 7);
      assert(!fs.readdirSync(working).some(name=>name.endsWith('.tmp')));
      assert.deepEqual(history(), original);
      fs.renameSync(photos,photos+'-offline');
      try {
        const displayed = await page.evaluate(async id => {
          const frame = await __TAURI__.core.invoke('preview',{id,large:true,bound:1600});
          const image = new Image();
          image.src = frame.inline_image || __TAURI__.core.convertFileSrc(frame.path);
          await image.decode();
          return [image.naturalWidth,image.naturalHeight];
        },ids[0]);
        assert(displayed.every(v=>v>0));
      } finally { fs.renameSync(photos+'-offline',photos); }
      console.log(JSON.stringify({recovery:'cancel, resume, forced exit, restart, offline decode and history',photos:count,
        workingBytes:sizes(working),remaining:fs.readdirSync(working)}));
    }
    assert.equal(warm.previews.generated,0);
    assert.equal(warm.previews.failed,0);
    assert.equal(available.length, rows.length);
    assert.equal(output.large.count, rows.length); assert.equal(output.thumbnails.count, rows.length);
    assert(importProgress && importProgress.ready === rows.length && !importProgress.failed && !importProgress.retained && !importProgress.paused && !importProgress.cancelled);
  } finally {
    clearInterval(sample); monitor?.kill();
    if (page) await page.evaluate(() => __TAURI__.core.invoke('close_window')).catch(()=>{});
    await browser?.close().catch(()=>{});
    await pause(700); if (child && child.exitCode === null) child.kill();
    await pause(450);
  }
}
(async () => {
  try {
    fs.mkdirSync(photos);
    const hashes = new Set(), inputs = [];
    for (const source of sources.flatMap(files).filter(file => raw.test(file)).sort()) {
      const hash = createHash('sha256').update(fs.readFileSync(source)).digest('hex');
      if (hashes.has(hash)) continue;
      hashes.add(hash);
      const name = inputs.length+'-'+path.basename(source);
      fs.copyFileSync(source, path.join(photos,name));
      inputs.push([name,hash,fs.statSync(source).size]);
    }
    assert(inputs.length);
    console.log(JSON.stringify({exe:path.resolve(executable),cpu:os.cpus()[0].model,ram:os.totalmem(),inputs}));
    const formats = (process.env.AE_BENCH_FORMATS || 'avif,jpeg,jpeg,avif').split(',');
    assert(formats.every(codec=>['avif','jpeg'].includes(codec)));
    for (const [i,codec] of formats.entries()) await round(i,codec==='avif');
    for (const [name,hash] of inputs) {
      assert.equal(createHash('sha256').update(fs.readFileSync(path.join(photos,name))).digest('hex'),hash);
    }
  } finally {
    assert.equal(path.dirname(fs.realpathSync(temp)), fs.realpathSync(os.tmpdir()));
    assert(path.basename(temp).startsWith('openroom-import-bench-'));
    try { fs.rmSync(temp,{recursive:true,force:true,maxRetries:12,retryDelay:300}); }
    catch (error) { console.error('Temporary benchmark directory',temp,error.message); }
  }
})().catch(e => { console.error(e); process.exitCode=1; });
