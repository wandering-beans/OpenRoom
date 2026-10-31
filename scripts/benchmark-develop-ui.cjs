// Real controls and decoded images; only catalog commands use the UI test's fixtures.
const assert = require('node:assert/strict');
const {writeFile} = require('node:fs/promises');
const adaptive = process.env.AE_ADAPTIVE_PREVIEW !== '0';

async function measure(page) {
  await page.click('#module-develop');
  await page.waitForFunction(() => document.querySelector('#full-image').src.startsWith('data:image/png') && document.querySelector('#full-image').naturalWidth > 0, null, {timeout:120000});
  if (process.env.AE_CONTINUOUS) return measureContinuous(page);
  const results = [];
  for (const bound of [1600, 0]) {
    if (bound === 0) {
      await page.click('#high-quality');
      await page.waitForFunction(() => calls.filter(c => c.command === 'develop_preview').at(-1)?.args.bound === 0 && window.realPreviewPending === 0, null, {timeout:120000});
    }
    for (const [name, detail, field] of [['tone',false,'exposure'], ['detail',true,'exposure'], ['detail',true,'contrast'], ['detail',true,'sharpen']]) {
      const values = {exposure:0,contrast:12,highlights:-25,shadows:20,vibrance:15,sharpen:detail ? 40 : 0,
        texture:detail ? 15 : 0,clarity:detail ? 10 : 0,noise_luma:detail ? 15 : 0};
      await page.evaluate(values => {
        for (const [key, value] of Object.entries(values)) {
          const input = document.getElementById(`develop-${key}`);
          input.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));
          input.value = value;
          input.dispatchEvent(new Event('input',{bubbles:true}));
          input.dispatchEvent(new Event('change',{bubbles:true}));
        }
      }, values);
      await page.waitForTimeout(600);
      await page.waitForFunction(() => window.realPreviewPending === 0 && document.querySelector('#full-image').complete, null, {timeout:120000});
      const samples = await page.evaluate(async ({field,bound,adaptive}) => {
        const slider = document.getElementById(`develop-${field}`);
        const image = document.getElementById('full-image');
        const dimensions = [image.naturalWidth, image.naturalHeight];
        slider.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));
        const samples = [];
        for (let index=0; index<25; index++) {
          const sample = await new Promise((resolve,reject) => {
            const timeout = setTimeout(() => reject(Error(document.getElementById('error').textContent || 'Preview did not load')), 120000);
            const started = performance.now();
            image.addEventListener('load', () => {
              const loaded = performance.now() - started;
              if ((!adaptive && (image.naturalWidth !== dimensions[0] || image.naturalHeight !== dimensions[1])) || calls.filter(c => c.command === 'develop_preview').at(-1).args.bound !== (adaptive ? 384 : bound)) {
                clearTimeout(timeout); reject(Error('Preview quality changed during gesture')); return;
              }
              requestAnimationFrame(() => requestAnimationFrame(() => {
                clearTimeout(timeout);
                const request = calls.filter(call => call.command === 'develop_preview').at(-1);
                resolve({loaded,paint:performance.now()-started,render:request.render_ms,request:request.end-request.start});
              }));
            }, {once:true});
            // Unique settings prevent disk-cache hits from masquerading as fast rendering.
            slider.value = String(field === 'exposure' ? (index - 12) * 0.05 : 30 + index);
            slider.dispatchEvent(new Event('input',{bubbles:true}));
          });
          if(index>=5) samples.push({...sample,dimensions});
        }
        slider.dispatchEvent(new Event('change',{bubbles:true}));
        return samples;
      }, {field,bound,adaptive});
      assert.equal(samples.length,20);
      const stats = key => {
        const values=samples.map(sample=>sample[key]).sort((a,b)=>a-b);
        return {median_ms:(values[9]+values[10])/2,p95_ms:values[18]};
      };
      results.push({bound,settings:name,slider:field,samples:20,dimensions:samples[0].dimensions,image_loaded:stats('loaded'),paint_opportunity:stats('paint'),render:stats('render'),request:stats('request'),timings:samples});
      console.log(JSON.stringify(results.at(-1)));
      await page.waitForTimeout(600);
      await page.waitForFunction(() => window.realPreviewPending === 0, null, {timeout:120000});
    }
  }
  assert.ok(await page.evaluate(adaptive => calls.filter(c=>c.command==='develop_preview').every(c=>[1600,0,...(adaptive?[384]:[])].includes(c.args.bound)), adaptive));
  const report = {surface:'Chromium, real frontend controls and Rust preview boundary; two animation frames after image load',results};
  if (process.env.AE_LATENCY_REPORT) await writeFile(process.env.AE_LATENCY_REPORT, JSON.stringify(report,null,2));
}
module.exports={measure};

async function measureContinuous(page) {
  const results = [];
  for (const bound of [1600, 0]) {
    if (bound === 0) await page.click('#high-quality');
    for (const detail of [false, true]) for (const field of ['exposure', 'contrast']) {
      await page.evaluate(detail => {
        for (const [field,value] of Object.entries({exposure:0,contrast:0,sharpen:detail?40:0,noise_luma:detail?15:0,texture:detail?15:0,clarity:detail?10:0})) {
          const slider=document.getElementById(`develop-${field}`);
          slider.value=value; slider.dispatchEvent(new Event('input', {bubbles:true})); slider.dispatchEvent(new Event('change', {bubbles:true}));
        }
      }, detail);
      await page.waitForTimeout(800);
      await page.waitForFunction(() => realPreviewPending===0 && document.getElementById('full-image').complete, null, {timeout:120000});
        const result=await page.evaluate(async ({bound,field,adaptive}) => {
          const image=document.getElementById('full-image');
          const dimensions=[image.naturalWidth,image.naturalHeight];
          const display=image.getBoundingClientRect();
          const input=document.getElementById(`develop-${field}`);
          const frames=[], started=performance.now(), firstCall=calls.length;
          let qualityChanged=false;
          const loaded=()=>{
            const current=image.getBoundingClientRect();
            qualityChanged ||= adaptive
              ? Math.abs(current.width-display.width)>2 || Math.abs(current.height-display.height)>2
              : image.naturalWidth!==dimensions[0]||image.naturalHeight!==dimensions[1];
            frames.push(performance.now()-started);
          };
          image.addEventListener('load',loaded);
          input.focus(); input.dispatchEvent(new PointerEvent('pointerdown'));
          for(let i=0;i<60;i++) {
            input.value=String(field === 'exposure' ? -1.45+i*.05 : -29+i);
            input.dispatchEvent(new Event('input', {bubbles:true}));
            await new Promise(resolve=>setTimeout(resolve,50));
          }
          const released=performance.now()-started;
          input.dispatchEvent(new Event('change', {bubbles:true})); input.blur();
          await new Promise((resolve,reject)=>{
            const deadline=performance.now()+120000;
            const check=()=>{
              if(performance.now()>deadline) return reject(Error('Final preview timed out'));
              if(realPreviewPending===0 && image.complete && performance.now()-started-released>800) return resolve();
              setTimeout(check,20);
            };check();
          });
          image.removeEventListener('load',loaded);
          if (qualityChanged || !frames.length) throw Error('Preview quality or progress failed');
          const requests=calls.slice(firstCall).filter(c=>c.command==='develop_preview');
          if(!requests.every(c=>c.args.bound===bound || (adaptive && c.args.bound===384)))throw Error('Unexpected preview bound');
          if(requests.at(-1)?.args.bound!==bound || image.naturalWidth!==dimensions[0] || image.naturalHeight!==dimensions[1])throw Error('Final quality was not restored');
          if(adaptive && !requests.some(c=>c.args.bound===384))throw Error('No interactive preview rendered');
          if(requests.at(-1)?.args.settings[field]!==Number(input.value))throw Error('Final slider value was not rendered');
          const during=frames.filter(t=>t<released);
          const gaps=during.slice(1).map((t,i)=>t-during[i]).sort((a,b)=>a-b);
          return {mode:'drag',bound,dimensions,inputs:60,input_duration_ms:released,frames:frames.length,during: during.length,
            first_frame_ms:frames[0]??null,release_to_last_frame_ms:Math.max(0,(frames.at(-1)??released)-released),
            frame_interval_median_ms:gaps.length?gaps[Math.floor(gaps.length/2)]:null,
            frame_interval_p95_ms:gaps.length?gaps[Math.floor(gaps.length*.95)]:null,
            requests:requests.length,cancellations:calls.slice(firstCall).filter(c=>c.command==='cancel_develop_preview').length};
        }, {bound,field,adaptive});
        results.push({...result,detail,slider:field}); console.log(JSON.stringify(results.at(-1)));
    }
  }
  await writeFile(process.env.AE_LATENCY_REPORT,JSON.stringify({surface:'Chromium + real Rust worker; image load timestamps during 60 continuous inputs',results},null,2));
}
