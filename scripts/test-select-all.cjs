const assert = require('node:assert/strict');

module.exports = async function testSelectAll(page, url) {
  const folder = '[data-folder="20261019撮影"] > .folder-label';
  const otherFolder = '[data-folder="20261020撮影"] > .folder-label';
  async function start() {
    await page.goto(url);
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('preview-1-1600'));
  }
  async function choose(selector, count) {
    await page.click(selector);
    await page.waitForFunction(count => document.querySelector('#result-count').textContent === `${count}枚の写真`, count);
  }
  await start();
  await choose(folder, '1,200');
  await page.keyboard.press('Meta+a');
  await page.waitForFunction(() => document.querySelector('#title').textContent === 'すべてのフォルダー'
    && document.querySelector('#result-count').textContent === '2,000枚の写真');
  assert.equal(await page.locator('#roots .folder-row:not(.active)').count(), 0, 'all folder rows, including collapsed children, must be selected');
  assert.equal(await page.locator('#roots .folder-label:not([aria-pressed="true"])').count(), 0);
  await choose(folder, '1,200');
  assert.equal(await page.locator('#roots .folder-row.active').count(), 1, 'a normal folder click must replace the selection');
  await page.locator('.film-tile.active-photo').click();
  const image = await page.getAttribute('#full-image', 'src');
  await page.evaluate(() => { window.invalidateSearchPages = 2; });
  await page.keyboard.press('Meta+a');
  await page.waitForFunction(() => document.querySelector('#selection-count').textContent.includes('1200枚選択'));
  assert.equal(await page.getAttribute('#full-image', 'src'), image);
  assert.equal(await page.locator('.film-tile:not(.selected)').count(), 0);
  await page.locator('.film-tile.active-photo').click({button:'right'});
  await page.click('#photo-export');
  assert.match(await page.textContent('#export-dialog'), /1200枚の写真/);
  await page.keyboard.press('Meta+a');
  assert.equal(await page.locator('#export-dialog').isVisible(), true, 'dialog shortcuts must not change background selection');
  await page.keyboard.press('Escape');
  await page.locator('.film-tile.active-photo').click();
  await page.keyboard.press('Control+f');
  await page.fill('#search', 'photo-0001');
  await page.keyboard.press('Meta+a');
  assert.deepEqual(await page.locator('#search').evaluate(node => [node.selectionStart, node.selectionEnd]), [0, 10], 'text inputs retain native select-all');

  // A failed page must not turn a full-selection request into a partial selection.
  await start();
  await page.locator('.film-tile.active-photo').click();
  await page.evaluate(() => { window.failSearchPage = true; });
  await page.keyboard.press('Control+a');
  await page.waitForFunction(() => !document.querySelector('#error').hidden);
  assert.match(await page.textContent('#selection-count'), /1枚選択/);
  await page.keyboard.press('Control+a');
  await page.waitForFunction(() => !document.querySelector('#error').hidden);
  assert.match(await page.textContent('#selection-count'), /1枚選択/, 'repeating after a failure must not select only the loaded page');
  await page.evaluate(() => { window.failSearchPage = false; });
  await page.keyboard.press('Control+a');
  await page.waitForFunction(() => document.querySelector('#selection-count').textContent.includes('2000枚選択'));
  assert.equal(await page.locator('#error').isVisible(), false);

  // Switching folders while a page is in flight must not select the new folder's photos.
  await start();
  await page.locator('.film-tile.active-photo').click();
  await page.evaluate(() => { window.holdSearchPage = true; });
  await page.keyboard.press('Meta+a');
  await page.waitForFunction(() => typeof window.releaseSearchPage === 'function');
  await choose(otherFolder, '800');
  await page.evaluate(() => window.releaseSearchPage());
  await page.waitForTimeout(150);
  assert.match(await page.textContent('#selection-count'), /1枚選択/);
  assert.equal(await page.locator('#error').isVisible(), false);
  await page.click('#module-develop');
  await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-'));
  await page.keyboard.press('Meta+a');
  await page.waitForFunction(() => document.querySelector('#selection-count').textContent.includes('800枚選択'));
  assert.equal(await page.locator('#roots .folder-row.active').count(), 1, 'Develop selects photos even when a folder was the last library target');
  console.log('Select all: folder/photo scope, all pages, stale cursor, export count, input/modal guards, failure and navigation cancellation passed.');
};
