const assert = require('node:assert/strict');

module.exports = async function testHistogram(page, url) {
  await page.goto(url);
  await page.evaluate(() => {
    window.histogramFixture = Array.from({length:3}, (_, channel) => Array.from({length:256}, (_, bin) =>
      bin === 0 ? 5 : bin === 255 ? 0 : Math.round(800 * Math.exp(-bin / 45) + 150 * Math.exp(-(((bin - 180 - channel * 14) / 9) ** 2)))));
  });
  await page.click('#module-develop');
  await page.waitForFunction(() => document.querySelector('#develop-clip-shadows').title === '黒つぶれ：あり');
  await page.waitForFunction(() => document.querySelector('[data-photo-field="焦点距離"]').textContent === '24 mm');
  assert.deepEqual(await page.locator('#develop-photo-info span').allTextContents(), ['ISO 400', '24 mm', 'f/2.8', '1/250 秒']);
  assert.equal(await page.getAttribute('#develop-clip-highlights', 'aria-label'), '白飛び：なし');
  assert.equal(await page.locator('#develop-clip-shadows').evaluate(node => node.style.color), 'rgb(220, 220, 220)');
  assert.ok((await page.getAttribute('#develop-hist-common', 'points')).length > 100);
  const before = await page.evaluate(() => calls.filter(call => call.command === 'develop_preview').length);
  await page.click('.histogram-panel summary');
  assert.equal(await page.locator('#develop-histogram').isVisible(), false);
  await page.click('.histogram-panel summary');
  assert.equal(await page.locator('#develop-histogram').isVisible(), true);
  assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'develop_preview').length), before, 'folding the histogram must not render the photo again');
  if (process.env.AE_UI_SCREENSHOT) await page.locator('.histogram-panel').screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png', '-histogram.png')});
  await page.evaluate(() => {
    window.histogramFixture = Array.from({length:3}, (_, channel) => Array.from({length:256}, (_, bin) => bin === 255 && channel === 0 ? 1 : 0));
  });
  await page.click('#film-next');
  await page.waitForFunction(() => document.querySelector('#develop-clip-highlights').title === '白飛び：あり');
  await page.waitForFunction(() => document.querySelector('[data-photo-field="ISO"]').textContent === 'ISO 400');
  assert.deepEqual(await page.locator('#develop-photo-info span').allTextContents(), ['ISO 400', '—', 'f/2.8', '—']);
  assert.equal(await page.getAttribute('#develop-clip-shadows', 'aria-label'), '黒つぶれ：なし');
  assert.equal(await page.locator('#develop-clip-highlights').evaluate(node => node.style.color), 'rgb(220, 35, 35)');
  assert.ok((await page.getAttribute('#develop-hist-common', 'points')).split(' ').every(point => point.endsWith(',90')), 'empty shared channel must stay on the baseline');
  console.log('Histogram: RGB overlap, clipping indicators, four metadata fields, missing values and collapse passed.');
};
