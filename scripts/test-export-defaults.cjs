const assert = require('node:assert/strict');
module.exports = async function testExportDefaults(page, url) {
  await page.evaluate(() => localStorage.removeItem('openroom.export.folder'));
  await page.goto(url);
  async function openExport() {
    await page.locator('.film-tile.selected').click({button:'right'});
    await page.click('#photo-export');
  }
  await openExport();
  assert.equal(await page.inputValue('#export-quality'), '100');
  assert.equal(await page.inputValue('#export-folder'), '');
  await page.click('#export-browse');
  assert.equal(await page.inputValue('#export-folder'), '/exports');
  await page.click('#export-start');
  await page.waitForFunction(() => document.querySelector('#export-result').textContent.includes('1/1'));
  const request = await page.evaluate(() => calls.filter(c=>c.command==='export_develop_batch').at(-1).args.request);
  assert.equal(request.folder, '/exports');
  assert.equal(request.options.quality, 100);
  await page.fill('#export-quality', '85');
  await page.click('#export-close');
  await openExport();
  assert.equal(await page.inputValue('#export-folder'), '/exports');
  assert.equal(await page.inputValue('#export-quality'), '85', 'keep explicit quality changes in the current session');
  await page.reload();
  await openExport();
  assert.equal(await page.inputValue('#export-folder'), '/exports', 'restart must restore the previous destination');
  assert.equal(await page.inputValue('#export-quality'), '100', 'new session defaults to maximum JPEG quality');
  await page.evaluate(() => { window.cancelExportFolder = true; });
  await page.click('#export-browse');
  assert.equal(await page.inputValue('#export-folder'), '/exports', 'cancel must preserve the previous destination');
  await page.evaluate(() => {
    window.cancelExportFolder = false;
    window.exportFolderFixture = '/exports/next';
  });
  await page.click('#export-browse');
  await page.reload();
  await openExport();
  assert.equal(await page.inputValue('#export-folder'), '/exports/next', 'a new selection replaces the remembered destination');
  await page.evaluate(() => {
    window.savedStorageSetItem = Storage.prototype.setItem;
    Storage.prototype.setItem = () => {throw new Error('storage unavailable');};
  });
  await page.click('#export-browse');
  assert.equal(await page.inputValue('#export-folder'), '/exports', 'storage failure must not prevent choosing a destination');
  await page.click('#export-start');
  await page.waitForFunction(() => document.querySelector('#export-result').textContent.includes('1/1'));
  await page.evaluate(() => {Storage.prototype.setItem = window.savedStorageSetItem;});
  await page.click('#export-close');
  console.log('Export defaults: quality 100, folder persistence, replacement, cancellation and unavailable storage passed.');
};
