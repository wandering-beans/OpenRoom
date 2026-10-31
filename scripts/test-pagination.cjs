const assert = require('node:assert/strict');

module.exports = async function testPagination(page, url) {
  async function start() {
    await page.goto(url);
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('preview-1-1600'));
  }
  async function nextPage() {
    await page.locator('#filmstrip').evaluate(node => { node.scrollLeft = 200 * 83; });
  }
  await start();
  await page.click('#toggle-inspector');
  await page.waitForFunction(() => !document.querySelector('#annotation-save').disabled);
  await page.fill('#annotation-memo', 'Unsaved pagination draft');
  const image = await page.getAttribute('#full-image', 'src');
  const startCall = await page.evaluate(() => { window.invalidateSearchPages = 3; return calls.length; });
  await nextPage();
  await page.waitForFunction(() => [...document.querySelectorAll('.film-number')].some(node => Number(node.textContent) > 200));
  assert.equal(await page.locator('#error').isVisible(), false);
  assert.equal(await page.locator('#discard-dialog').isVisible(), false);
  assert.equal(await page.getAttribute('#full-image', 'src'), image, 'retries must retain the selected photo');
  assert.equal(await page.inputValue('#annotation-memo'), 'Unsaved pagination draft');
  assert.deepEqual(await page.evaluate(start => calls.slice(start).filter(c => c.command === 'search').map(c => c.args.cursor?.offset || 0), startCall),
    [200, 0, 200, 0, 200, 0, 200], 'every invalid generation must restart from page one');
  assert.equal(await page.locator('.film-tile').evaluateAll(tiles => new Set(tiles.map(tile => tile.title)).size === tiles.length), true);

  await start();
  await page.evaluate(() => { window.invalidateSearchPages = 100; });
  await nextPage();
  await page.waitForFunction(() => window.invalidateSearchPages < 99);
  await page.keyboard.press('Control+f');
  await page.fill('#search', 'photo-0001');
  await page.waitForFunction(() => document.querySelector('#result-count').textContent === '1枚の写真');
  const afterSwitch = await page.evaluate(() => calls.filter(c => c.command === 'search').length);
  await page.waitForTimeout(1200); // Exceed the retry delay to catch obsolete requests resuming.
  assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'search').length), afterSwitch,
    'changing the query must stop the previous retry loop');
  assert.equal(await page.locator('#error').isVisible(), false);

  await start();
  const errorStart = await page.evaluate(() => { window.failSearchPage = true; return calls.length; });
  await nextPage();
  await page.waitForFunction(() => !document.querySelector('#error').hidden);
  assert.match(await page.textContent('#error'), /search unavailable/);
  assert.equal(await page.evaluate(start => calls.slice(start).filter(c => c.command === 'search').length, errorStart), 1,
    'unrelated search failures must not be silently retried');
  console.log('Pagination: repeated invalidation, retained photo/draft, query cancellation and unrelated failures passed.');
};
