const assert = require('node:assert/strict');

module.exports = async function testCatalogRemoval(page) {
  await page.reload();
  const first = page.locator('.film-tile[title="photo-0000.jpg"]');
  const second = page.locator('.film-tile[title="photo-0001.jpeg"]');
  await first.waitFor();
  await first.click({button:'right'});
  await page.click('#catalog-remove');
  await page.locator('#discard-dialog[open]').waitFor();
  assert.match(await page.locator('#discard-message').textContent(), /原本ファイルは削除しません/);
  await page.click('#discard-dialog button[value="cancel"]');
  assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'remove_from_catalog').length), 0);

  await first.click();
  await second.click({modifiers:['Shift']});
  await first.click({button:'right'});
  await page.click('#catalog-remove');
  await page.locator('#discard-dialog[open]').waitFor();
  assert.match(await page.locator('#discard-message').textContent(), /2枚の写真/);
  await page.click('#discard-dialog button[value="discard"]');
  await page.waitForFunction(() => document.querySelector('#catalog-count').textContent === '1,998');
  assert.equal(await first.count(), 0);
  assert.equal(await second.count(), 0);
  assert.deepEqual(await page.evaluate(() => calls.find(call => call.command === 'remove_from_catalog').args.selection), {kind:'photos',ids:[1,2]});

  const nested = page.locator('.folder-row[data-folder="20261019撮影/選定"] .folder-label');
  await page.locator('.folder-row[data-folder="20261019撮影"] .folder-toggle').click();
  await nested.click();
  await page.waitForFunction(() => document.querySelector('#result-count').textContent === '398枚の写真');
  await nested.focus();
  await page.keyboard.press('Shift+F10');
  await page.locator('#photo-menu').waitFor({state:'visible'});
  assert.equal(await page.locator('#photo-export').isVisible(), false);
  await page.keyboard.press('Enter');
  await page.locator('#discard-dialog[open]').waitFor();
  await page.click('#discard-dialog button[value="discard"]');
  await page.waitForFunction(() => document.querySelector('#catalog-count').textContent === '1,600');
  assert.equal(await nested.count(), 0);
  assert.equal(await page.locator('.folder-row[data-folder="20261020撮影"]').count(), 1);

  await page.click('#module-develop');
  await page.locator('#develop-controls:not([disabled])').waitFor();
  await page.evaluate(() => { window.failRemoval = true; });
  await page.locator('.film-tile.active-photo').click({button:'right'});
  await page.click('#catalog-remove');
  await page.locator('#discard-dialog[open]').waitFor();
  await page.click('#discard-dialog button[value="discard"]');
  await page.waitForFunction(() => !document.querySelector('#error').hidden && document.querySelector('#error').textContent.includes('Injected catalog removal failure'));
  assert.equal(await page.locator('#catalog-count').textContent(), '1,600');
  await page.locator('#develop-controls:not([disabled])').waitFor();
  await page.evaluate(() => { window.failRemoval = false; });

  await page.click('#module-library');
  await page.locator('.folder-row[data-folder=""] .folder-label').click({button:'right'});
  await page.click('#catalog-remove');
  await page.locator('#discard-dialog[open]').waitFor();
  await page.click('#discard-dialog button[value="discard"]');
  await page.waitForFunction(() => document.querySelector('#catalog-count').textContent === '0' && document.querySelector('#result-count').textContent === '0枚の写真');
  assert.equal(await page.locator('#roots .folder-row').count(), 0);
  assert.equal(await page.locator('.film-tile').count(), 0);
  assert.equal(await page.locator('#full-image').isVisible(), false);
};
