// Exercise native range events as well as the real RAW worker, including the first drag.
const assert = require('node:assert/strict');
const {writeFile} = require('node:fs/promises');
async function measure(page) {
  await page.click('#module-develop');
  const idle = () => page.waitForFunction(() => realPreviewPending === 0 && document.querySelector('#full-image').src.startsWith('data:image/png') && document.querySelector('#full-image').complete, null, {timeout:120000});
  await idle();
  await page.waitForTimeout(2500);
  const results = [];
  for (const field of ['exposure','exposure','raw_temperature','raw_tint','contrast']) {
    const slider = page.locator(`#develop-${field}`);
    await slider.scrollIntoViewIfNeeded();
    const box = await slider.boundingBox();
    await page.evaluate(field => {
      const image = document.getElementById('full-image');
      const started = performance.now();
      window.dragMeasure = {started, frames:[], inputs:[], events:[], callStart:calls.length};
      window.dragLoaded = () => dragMeasure.frames.push(performance.now()-started);
      image.addEventListener('load', dragLoaded);
      const input = document.getElementById(`develop-${field}`);
      for (const type of ['input','change','pointerup','pointerdown','focus','blur']) input.addEventListener(type, event => {
        dragMeasure.events.push({type:event.type, time:performance.now()-dragMeasure.started});
        if (event.type==='input') dragMeasure.inputs.push(Number(input.value));
      }, {signal:(window.dragAbort = window.dragAbort || new AbortController()).signal});
    }, field);
    await page.mouse.move(box.x+box.width*.3, box.y+box.height/2);
    await page.mouse.down();
    for (let i=0; i<60; i++) {
      await page.mouse.move(box.x+box.width*(.3+.4*i/59), box.y+box.height/2);
      await page.waitForTimeout(16);
    }
    await page.mouse.up();
    const released = await page.evaluate(() => performance.now()-dragMeasure.started);
    await idle();
    await page.waitForTimeout(900);
    await idle();
    const result = await page.evaluate(({field,released}) => {
      const m=dragMeasure, image=document.getElementById('full-image');
      image.removeEventListener('load',dragLoaded); dragAbort.abort(); window.dragAbort=null;
      const requests=calls.slice(m.callStart).filter(c=>c.command==='develop_preview');
      const during=m.frames.filter(t=>t<released);
      const gaps=during.slice(1).map((v,i)=>v-during[i]).sort((a,b)=>a-b);
      return {field,inputs:m.inputs.length,first_frame_ms:m.frames[0],during:during.length,
        frame_interval_median_ms:gaps[Math.floor(gaps.length/2)],frame_interval_p95_ms:gaps[Math.floor(gaps.length*.95)],
        release_to_last_frame_ms:Math.max(0,m.frames.at(-1)-released),input_duration_ms:released,
        final_value:Number(document.getElementById(`develop-${field}-value`).value),
        final_rendered:requests.at(-1)?.args.settings[field], final_bound:requests.at(-1)?.args.bound,
        bounds:requests.map(r=>r.args.bound),requests:requests.map(r=>({bound:r.args.bound,start:r.start-m.started,end:r.end-m.started})),events:m.events};
    }, {field,released});
    assert.ok(result.inputs>10, 'Native pointer drag must generate inputs');
    assert.equal(result.final_bound,1600, 'Release must restore display quality');
    assert.equal(result.final_value,result.final_rendered, 'Final render must use the final input');
    results.push(result); console.log(JSON.stringify(result));
  }
  await writeFile(process.env.AE_LATENCY_REPORT,JSON.stringify({surface:`${process.env.AE_WEBKIT?'WebKit':'Chromium'} native pointer events + real RAW worker; image load timestamps`,idle_after_initial_preview_ms:2500,results},null,2));
}
module.exports={measure};
