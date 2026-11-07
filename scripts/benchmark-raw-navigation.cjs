// Called by benchmark-raw-import with AE_BENCH_UI=1. Operates on its disposable catalog.
const { DatabaseSync } = require('node:sqlite');
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const wait = ms => new Promise(resolve => setTimeout(resolve,ms));

module.exports = async function measure(page, previews, addBatchRoot) {
  const catalog = new DatabaseSync(previews.slice(0,-'.previews'.length), {readOnly:true,timeout:15000});
  let all;
  try { all = catalog.prepare('select id,name from items order by name').all(); }
  finally { catalog.close(); }
  const nikon = all.filter(r => /-\d\d\.nef$/i.test(r.name));
  const targets = (nikon.length >= 3 ? nikon : all).slice(0,3);
  const results = [];
  const immediate = process.env.AE_BENCH_IMMEDIATE === '1';
  const conditions = (process.env.AE_BENCH_CONDITIONS || 'prefetched,cold,importing').split(',');
  for (const condition of conditions) {
    for (const photo of targets) {
      await page.locator('#grid-view').click();
      await page.keyboard.press('Control+f');
      await page.locator('#search').fill(photo.name);
      const tile = page.locator('.tile[title='+JSON.stringify(photo.name)+']').first();
      await tile.waitFor();
      await tile.click();
      await page.waitForFunction(name => document.querySelector('.tile.active-photo')?.title === name,photo.name);
      await page.evaluate(() => __TAURI__.core.invoke('set_develop_prefetch',{ids:[]}));
      if (condition === 'prefetched') {
        await page.evaluate(id => __TAURI__.core.invoke('set_develop_prefetch',{ids:[id]}),photo.id);
        const working = path.join(previews,'working');
        let complete = false;
        for (let attempt=0;attempt<300;attempt++) {
          complete = fs.readdirSync(working).some(d => d.startsWith('develop-prefetch-') &&
            fs.existsSync(path.join(working,d,'preview.png')));
          if (complete) break;
          await wait(100);
        }
        assert(complete, 'Prefetch did not complete');
        await wait(350);
      }
      if (condition === 'importing') {
        // Import a separate unindexed root while navigating the already indexed photos.
        const rootId = addBatchRoot('background-');
        await page.evaluate(rootId => {
          window.benchBusy = true;
          window.benchBackground = __TAURI__.core.invoke('scan',{rootId}).finally(()=>{ window.benchBusy = false; });
        },rootId);
      }
      await page.evaluate(() => {
        window.benchFrame = {};
        const frame = window.benchFrame;
        frame.bulkAtSwitch = window.benchBusy === true;
        document.querySelector('#module-develop').addEventListener('click',() => {
          frame.start = performance.now();
          function observe() {
            const img = document.querySelector('#full-image');
            const now = performance.now() - frame.start;
            if (frame.image == null && img.complete && img.naturalWidth && img.checkVisibility() &&
                !document.querySelector('#viewer').hidden) frame.image = now;
            if (frame.controls == null && !document.querySelector('#develop-exposure').matches(':disabled') &&
                !document.querySelector('#develop-panel').hidden) frame.controls = now;
            if (frame.image == null || frame.controls == null) requestAnimationFrame(observe);
          }
          requestAnimationFrame(observe);
        }, {capture:true,once:true});
      });
      await page.locator('#module-develop').click();
      await page.waitForFunction(() => window.benchFrame.controls != null);
      if (!immediate) {
        await page.waitForFunction(() => window.benchFrame.image != null && document.querySelector('#develop-histogram').hasAttribute('title'));
        await wait(4000);
      }
      const slider = page.locator('#develop-exposure');
      await slider.evaluate(el => {
        for (let p=el.parentElement;p;p=p.parentElement) if (p.tagName==='DETAILS') p.open=true;
        el.scrollIntoView({block:'center'});
        const img = document.querySelector('#full-image');
        const src = img.src;
        el.addEventListener('input', () => {
          const start = performance.now();
          window.benchFrame.bulkAtInput = window.benchBusy === true;
          function observe() {
            if (img.src !== src && img.complete && img.naturalWidth && Math.max(img.naturalWidth,img.naturalHeight) <= 384 && img.checkVisibility()) {
              window.benchFrame.input = performance.now()-start;
            } else requestAnimationFrame(observe);
          }
          requestAnimationFrame(observe);
        }, {once:true});
      });
      const box = await slider.boundingBox();
      assert(box, 'Exposure is not visible');
      const fraction = Number(await slider.inputValue()) > 0 ? .38 : .62;
      await page.mouse.move(box.x + box.width * fraction, box.y+box.height/2);
      await page.mouse.down();
      try { await page.waitForFunction(() => window.benchFrame.input != null, undefined,{timeout:30000}); }
      finally { await page.mouse.up(); }
      const value = await page.evaluate(() => ({...window.benchFrame, exposure:document.querySelector('#develop-exposure').value}));
      results.push({condition,immediate,photo:photo.name,...value});
      console.log(JSON.stringify({developTrial:results.at(-1)}));
      await page.locator('#module-library').click();
      await page.waitForFunction(() => !document.body.classList.contains('develop-mode'));
      if (condition === 'importing') {
        const jobs = await page.evaluate(() => window.benchBackground);
        assert(jobs.previews.generated > 0 && jobs.previews.failed === 0 && value.bulkAtSwitch && value.bulkAtInput,
          `Both actions must overlap real background generation: ${JSON.stringify({jobs:jobs.previews,value})}`);
      }
    }
  }
  for (const condition of conditions) {
    const values = results.filter(r => r.condition === condition);
    const stat = field => {
      const sorted = values.map(r => r[field]).sort((a,b)=>a-b);
      return {mean:sorted.reduce((a,b)=>a+b,0)/sorted.length,p95:sorted[Math.ceil(.95*sorted.length)-1]};
    };
    console.log(JSON.stringify({condition,immediate,n:values.length,image:stat('image'),controls:stat('controls'),input:stat('input')}));
  }
};
