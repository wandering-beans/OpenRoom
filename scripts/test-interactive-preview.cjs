const assert = require('node:assert/strict');

module.exports = async function testInteractivePreview(page) {
  await page.click('#module-develop');
  await page.waitForFunction(() => document.getElementById('full-image').src.includes('develop-1-0-1600'));
  for (const bound of [1600, 0]) {
    if (bound === 0) await page.click('#zoom-in');
    await page.waitForFunction(bound => calls.filter(c => c.command === 'develop_preview').at(-1)?.args.bound === bound, bound);
    await page.waitForFunction(bound => document.getElementById('full-image').src.endsWith(`-${bound}`) && document.getElementById('full-image').complete, bound);
    const before = await page.locator('#full-image').boundingBox();
    await page.evaluate(() => {
      const slider = document.getElementById('develop-exposure');
      slider.dispatchEvent(new PointerEvent('pointerdown', {bubbles:true}));
      slider.value = '0.5';
      slider.dispatchEvent(new Event('input', {bubbles:true}));
    });
    await page.waitForFunction(() => document.getElementById('full-image').src.includes('develop-1-0.5-384'));
    const during = await page.locator('#full-image').boundingBox();
    assert.ok(Math.abs(before.width-during.width) < 1 && Math.abs(before.height-during.height) < 1,
      'interactive preview must preserve fitted size and 100% zoom');
    await page.evaluate(() => {
      const slider = document.getElementById('develop-exposure');
      slider.value = '0.75'; slider.dispatchEvent(new Event('input', {bubbles:true}));
      slider.dispatchEvent(new Event('change', {bubbles:true}));
    });
    await page.waitForFunction(bound => document.getElementById('full-image').src.includes(`develop-1-0.75-${bound}`), bound);
    await page.waitForFunction(() => calls.some(c => c.command === 'save_develop' && c.args.settings.exposure === 0.75));
    assert.equal(Number(await page.inputValue('#develop-exposure')), 0.75);
  }
  // A slow full-resolution request must not hold up a new drag or replace its newer image.
  await page.evaluate(() => {window.holdDevelopPreview = true;});
  await page.click('#high-quality');
  await page.waitForFunction(() => typeof window.releaseDevelopPreview === 'function');
  await page.evaluate(() => {
    window.holdDevelopPreview = false;
    const slider = document.getElementById('develop-exposure');
    slider.dispatchEvent(new PointerEvent('pointerdown', {bubbles:true}));
    slider.value = '1'; slider.dispatchEvent(new Event('input', {bubbles:true}));
  });
  await page.waitForFunction(() => document.getElementById('full-image').src.includes('develop-1-1-384'));
  assert.ok(await page.evaluate(() => calls.some(c => c.command === 'cancel_develop_preview')));
  await page.evaluate(() => {window.releaseDevelopPreview();});
  await page.evaluate(() => document.getElementById('develop-exposure').dispatchEvent(new Event('change', {bubbles:true})));
  await page.waitForFunction(() => document.getElementById('full-image').src.includes('develop-1-1-0'));
  console.log('Interactive preview: lightweight drag, final quality, stable zoom, and superseded render checks passed.');
};
