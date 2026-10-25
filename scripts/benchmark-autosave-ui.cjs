// Real autosave, SQLite, preview encoding and resident worker; an isolated test catalog.
const assert = require('node:assert/strict');
const {writeFile} = require('node:fs/promises');
module.exports.measure = async page => {
  await page.click('#module-develop');
  await page.waitForFunction(()=>document.querySelector('#full-image').src.startsWith('data:image/png') && realPreviewPending===0, null, {timeout:120000});
  const results=[];
  for(const bound of [1600,0]) {
    if(bound===0) await page.click('#high-quality');
    for(const detail of [false,true]) {
      await page.evaluate(detail=>{
        for(const [key,value] of Object.entries({sharpen:detail?40:0,noise_luma:detail?15:0,texture:detail?15:0,clarity:detail?10:0})) {
          const input=document.getElementById(`develop-${key}`); input.value=value;
          input.dispatchEvent(new Event('input', {bubbles:true})); input.dispatchEvent(new Event('change', {bubbles:true}));
        }
      },detail);
      await page.waitForTimeout(800);
      await page.waitForFunction(bound=>document.querySelector('#develop-save').disabled && realPreviewPending===0 && document.querySelector('#full-image').complete && calls.filter(c=>c.command==='develop_preview').at(-1)?.args.bound===bound, bound,{timeout:120000});
      const timings=[], traces=[];
      for(let sample=0;sample<12;sample++) {
        const result=await page.evaluate(async sample=>{
          const slider=document.getElementById('develop-exposure'), image=document.getElementById('full-image');
          const sleep=ms=>new Promise(r=>setTimeout(r,ms));
          const wait=async predicate=>{const deadline=performance.now()+120000;while(!predicate()){if(performance.now()>deadline)throw Error(document.getElementById('error').textContent||'Autosave benchmark timeout');await sleep(2);}};
          const dimensions=[image.naturalWidth,image.naturalHeight];
          const set=value=>{slider.value=value;slider.dispatchEvent(new Event('input', {bubbles:true}));};
          const before=calls.filter(c=>c.command==='save_develop').length;
          set(-1.5+sample*.1); slider.dispatchEvent(new Event('change', {bubbles:true}));
          await wait(()=>calls.filter(c=>c.command==='save_develop').length>before);
          // Resume as autosave starts, reproducing short adjustments separated by a pause.
          const start=performance.now();
          const target=Number((0.5+sample*.1).toFixed(3));
          const loaded=new Promise(resolve=>{const listener=()=>{if(Math.abs(realRenderedExposure-target)>0.0001)return;image.removeEventListener('load',listener);requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(performance.now()-start)));};image.addEventListener('load',listener);});
          set(0.5+sample*.1);
          const milliseconds=await loaded;
          const during=calls.filter(c=>c.command==='save_develop').length;
          slider.dispatchEvent(new Event('change', {bubbles:true}));
          await wait(()=>document.getElementById('develop-save').disabled && realPreviewPending===0);
          return {trace:calls.filter(c=>c.start>=start-20 && ['save_develop','develop_preview'].includes(c.command)).map(c=>({command:c.command,start:c.start-start,duration:c.end-c.start,render_ms:c.render_ms})),milliseconds,dimensions:[image.naturalWidth,image.naturalHeight],expected:dimensions,savesWhileHeld:during-before-1};
        },sample);
        assert.deepEqual(result.dimensions,result.expected);
        assert.equal(result.savesWhileHeld,0,'Autosave must not finish the new gesture');
        if(sample>=2){timings.push(result.milliseconds);traces.push(result.trace);}
      }
      const sorted=timings.toSorted((a,b)=>a-b);
      results.push({bound,detail,samples:timings.length,median_ms:(sorted[4]+sorted[5])/2,p95_ms:sorted[9],timings,traces});
      console.log(JSON.stringify(results.at(-1)));
    }
  }
  await writeFile(process.env.AE_LATENCY_REPORT,JSON.stringify({surface:'Chromium + real save/SQLite/library generation + Rust preview; resume input at autosave start; two animation frames after load',results},null,2));
};
