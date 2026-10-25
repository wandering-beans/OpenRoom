const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const path = require('node:path');

async function run() {
  const {estimateStraighteningAngle} = await import('../desktop/ui/straighten.ts');
  function fixture(angle, vertical = false) {
    const width = 320, height = 240, data = new Uint8ClampedArray(width * height * 4);
    const slope = Math.tan(angle * Math.PI / 180);
    for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
      const distance = vertical ? x - width / 2 + (y - height / 2) * slope : y - height / 2 - (x - width / 2) * slope;
      const value = Math.max(0, Math.min(255, 128 + distance * 100));
      const i = (y * width + x) * 4;
      data.set([value, value, value, 255], i);
    }
    return {data, width, height};
  }
  for (const vertical of [false, true]) for (const angle of [-10, -4, 0, 3, 9]) {
    const actual = estimateStraighteningAngle(fixture(angle, vertical));
    assert.ok(actual !== null && Math.abs(actual - angle) <= .75, `edge ${angle}, vertical=${vertical}: ${actual}`);
  }
  const blank = fixture(0); blank.data.fill(255);
  assert.equal(estimateStraighteningAngle(blank), null, 'blank images must not invent a horizon');
  assert.equal(estimateStraighteningAngle(fixture(20)), null, 'out-of-range angles need manual correction');
  const border = fixture(0);
  for (let y = 0; y < border.height; y++) for (let x = 0; x < border.width; x++) {
    border.data.set([255, 255, 255, y > 30 + x * .1 ? 255 : 0], (y * border.width + x) * 4);
  }
  assert.equal(estimateStraighteningAngle(border), null, 'transparent image edges are not horizons');
  let seed = 17;
  for (let i = 0; i < blank.data.length; i += 4) {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    blank.data.set([seed & 255, seed >>> 8 & 255, seed >>> 16 & 255, 255], i);
  }
  assert.equal(estimateStraighteningAngle(blank), null, 'noise must not invent a horizon');
  console.log('Automatic straightening: signed horizontal/vertical edges, blank, noise and unsupported angles passed.');
}
run().catch(error => {console.error(error); process.exitCode = 1;});
