const assert = require('node:assert/strict');

module.exports = async function testInteractivePreview(page) {
  await page.evaluate(() => {
    window.photoMetadataGate = new Promise(resolve => { window.releasePhotoMetadata = resolve; });
  });
  await page.click('#toggle-inspector');
  await page.waitForFunction(() => calls.some(c => c.command === 'photo_metadata' && c.args.id === 1));
  await page.click('#module-develop');
  await page.waitForFunction(() => document.getElementById('full-image').src.includes('develop-1-0-1600'));
  await page.waitForFunction(() => calls.some(c => c.command === 'develop_preview' && c.args.bound === 384));
  const metadataReads = () => page.evaluate(() => calls.filter(c => c.command === 'photo_metadata' && c.args.id === 1).length);
  assert.equal(await metadataReads(), 1, 'Develop joins the pending library metadata read');
  await page.evaluate(() => window.releasePhotoMetadata());
  await page.waitForFunction(() => document.querySelector('#develop-photo-info').textContent.includes('ISO 400'));
  await page.waitForTimeout(100);
  assert.ok((await page.locator('#full-image').getAttribute('src')).includes('develop-1-0-1600'),
    'warming the drag worker must not replace the displayed high-quality preview');
  for (const bound of [1600, 0]) {
    if (bound === 0) await page.click('#zoom-in');
    await page.waitForFunction(bound => document.getElementById('full-image').src.endsWith(`-${bound}`) && document.getElementById('full-image').complete, bound);
    const before = await page.locator('#full-image').boundingBox();
    await page.evaluate(() => {
      const slider = document.getElementById('develop-exposure');
      window.gestureCallStart = calls.length;
      slider.focus();
      slider.dispatchEvent(new PointerEvent('pointerdown', {bubbles:true,pointerId:7}));
      slider.value = '0.5';
      slider.dispatchEvent(new Event('input', {bubbles:true}));
      slider.blur(); // WebKit blurs ranges after mouse-down while the drag is still active.
    });
    await page.waitForFunction(() => document.getElementById('full-image').src.includes('develop-1-0.5-384'));
    const during = await page.locator('#full-image').boundingBox();
    assert.ok(Math.abs(before.width-during.width) < 1 && Math.abs(before.height-during.height) < 1,
      'interactive preview must preserve fitted size and 100% zoom');
    assert.ok(await page.evaluate(() => calls.slice(gestureCallStart).filter(c=>c.command==='develop_preview').every(c=>c.args.bound===384)),
      'element blur during a pointer drag must not start a high-quality render');
    await page.evaluate(() => {
      const slider = document.getElementById('develop-exposure');
      slider.value = '0.75'; slider.dispatchEvent(new Event('input', {bubbles:true}));
      window.dispatchEvent(new PointerEvent('pointerup', {pointerId:7}));
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
  // A warmup finishing after leaving Develop must not change the library image or show an error.
  await page.click('#module-library');
  await page.evaluate(() => { window.holdDevelopPreview = true; window.releaseDevelopPreview = null; });
  await page.click('#module-develop');
  await page.waitForFunction(() => typeof window.releaseDevelopPreview === 'function');
  await page.evaluate(() => { const release=window.releaseDevelopPreview; window.releaseDevelopPreview=null; release(); });
  await page.waitForFunction(() => typeof window.releaseDevelopPreview === 'function' && calls.filter(c=>c.command==='develop_preview').at(-1).args.bound===384);
  await page.click('#module-library');
  await page.waitForFunction(() => document.getElementById('full-image').src.includes('#preview-'));
  const libraryImage = await page.locator('#full-image').getAttribute('src');
  await page.evaluate(() => { window.holdDevelopPreview=false; window.releaseDevelopPreview(); });
  await page.waitForTimeout(100);
  assert.equal(await page.locator('#full-image').getAttribute('src'), libraryImage);
  assert.equal(await page.locator('#error').isVisible(), false);
  await page.click('#toggle-inspector');
  await page.waitForFunction(() => document.querySelector('#photo-details').textContent.includes('Camera 1'));
  assert.equal(await metadataReads(), 1, 'editing a photo preserves its original EXIF');
  // While reads are blocked, select B, C, then B again. Native code cancels the first B.
  await page.evaluate(() => {
    window.photoMetadataGate = new Promise(resolve => { window.releasePhotoMetadata = resolve; });
  });
  const start = await page.evaluate(() => calls.length);
  for (const [id, button] of [[2,'film-next'], [3,'film-next'], [2,'film-previous']]) {
    await page.click('#' + button);
    await page.waitForFunction(id => calls.filter(c => c.command === 'photo_metadata').at(-1).args.id === id, id);
  }
  await page.evaluate(() => window.releasePhotoMetadata());
  await page.waitForFunction(() => document.querySelector('#photo-details').textContent.includes('Camera 2'));
  assert.deepEqual(await page.evaluate(start => calls.slice(start).filter(c => c.command === 'photo_metadata').map(c => c.args.id), start), [2,3,2]);
  console.log('Interactive preview: lightweight drag, final quality, stable zoom, and superseded render checks passed.');
};
