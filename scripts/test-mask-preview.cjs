const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const path = require('node:path');

(async () => {
  const {maskCoverage} = await import('../desktop/ui/mask-preview.ts');
  const shape = {kind:'linear', operation:'add', start:[0,0], end:[0,.5], radius:.15, feather:.5, opacity:1, points:[]};
  const mask = {shapes:[shape], invert:false};
  assert.deepEqual([...maskCoverage(mask, 4, 4)], [.75,.75,.75,.75,.25,.25,.25,.25,0,0,0,0,0,0,0,0]);
  for (const kind of ['linear','radial','brush']) {
    const selection = {...shape, kind, start:[.5,.5], end:[.8,.8], points:[[.3,.5],[.7,.5]]};
    const original = maskCoverage({shapes:[selection]}, 64, 32);
    assert.ok(original.some(v => v > .9)); assert.ok(original.some(v => v === 0));
    for (const operation of ['add','subtract','intersect']) {
      const combined = maskCoverage({shapes:[selection,{...selection,operation}]}, 64, 32);
      for (let i=0; i<original.length; i++) {
        const expected = operation==='add' ? original[i] : operation==='subtract' ? original[i]*(1-original[i]) : original[i]**2;
        assert.ok(Math.abs(combined[i]-expected)<1e-6, `${kind} ${operation}`);
      }
    }
    const inverted = maskCoverage({shapes:[selection],invert:true}, 64, 32);
    for (let i=0; i<original.length; i++) assert.ok(Math.abs(inverted[i]+original[i]-1)<1e-6);
  }
  const dot = {...shape,kind:'brush',points:[[.5,.5]],radius:.2,feather:0};
  const pixels = maskCoverage({shapes:[dot]}, 80, 40);
  const horizontal = [...pixels.slice(20*80,21*80)].filter(v=>v>0).length;
  const vertical = Array.from({length:40},(_,y)=>pixels[y*80+40]).filter(v=>v>0).length;
  assert.equal(horizontal,vertical,'a brush must stay round on a non-square photo');
  assert.equal(maskCoverage({shapes:[{...shape,kind:'color'}]},4,4),null,'retain legacy masks without inventing a selection preview');
  console.log('Mask overlay coverage: gradients, brush aspect, add/subtract/intersection and inversion passed.');
})().catch(error=>{console.error(error);process.exitCode=1;});
