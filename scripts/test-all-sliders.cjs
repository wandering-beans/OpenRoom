// Exercise the shipped controls through WebKit/Chromium; pixel effects are checked by the engine audit.
const assert = require('node:assert/strict');
const { writeFile } = require('node:fs/promises');

module.exports = async function testAllSliders(page) {
  const results = [];
  await page.evaluate(() => { window.rawFixture = true; window.autoCaFixture = true; });
  await page.click('#module-develop');
  await page.locator('#develop-controls:not([disabled])').waitFor();
  await page.locator('#develop-raw_temperature:visible').waitFor();
  const inventory = await page.locator('input[type=range]').evaluateAll(inputs => inputs.map(input => ({
    id: input.id, min: Number(input.min), max: Number(input.max), step: Number(input.step || 1),
    legacy: Boolean(input.closest('#develop-curves')),
  })));
  assert.equal(new Set(inventory.map(input => input.id)).size, inventory.length);

  async function reveal(id) {
    if (id.startsWith('develop-crop-') || id === 'develop-angle') {
      await page.locator('[data-tool="crop"]').click();
    } else if (!id.startsWith('local-') && !id.startsWith('mask-')) {
      await page.locator('[data-tool=""]').click();
    }
    await page.locator(`#${id}`).evaluate(input => {
      for (let parent = input.parentElement; parent; parent = parent.parentElement) {
        if (parent.tagName === 'DETAILS' && !parent.open) parent.querySelector(':scope > summary').click();
      }
    });
    if (id.startsWith('develop-mixer-')) await page.selectOption('#develop-mixer-channel', id.split('-')[2]);
    if (id.startsWith('develop-grading-')) await page.click(`[data-grading-mode="${id.split('-')[2]}"]`);
    if (id.startsWith('develop-defringe_')) await page.check('#develop-defringe');
    await page.locator(`#${id}`).scrollIntoViewIfNeeded();
    assert.ok(await page.locator(`#${id}`).isVisible(), `${id} must be reachable in the UI`);
    assert.ok(await page.locator(`#${id}`).isEnabled(), `${id} must be enabled with its prerequisites`);
  }
  async function savedSettings() {
    await page.locator('#module-develop').focus();
    if (await page.locator('#develop-save').isEnabled()) await page.click('#develop-save');
    await page.waitForFunction(() => document.querySelector('#develop-save').disabled);
    return page.evaluate(async () => (await __TAURI__.core.invoke('get_develop', {id:1})).settings);
  }
  function valueIn(recipe, id, raster = false) {
    if (id.startsWith('local-')) return recipe.masks[0].adjustments[id.slice(6)] ?? 0;
    if (id === 'mask-amount') return recipe.masks[0].amount;
    if (id === 'mask-radius' || id === 'mask-feather') return recipe.masks[0].shapes.at(-1)[id.slice(5)];
    let path = id.replace('develop-', '').split('-');
    if (path[0].startsWith('raw_') && raster) path[0] = path[0].replace('raw_', 'raster_');
    const value = path.reduce((value, key) => value[key], recipe);
    return path[0] === 'crop' ? value * 100 : value;
  }
  const close = (actual, expected, context) => assert.ok(Math.abs(actual - expected) < .001, `${context}: ${actual} != ${expected}`);
  async function check(control, raster = false) {
    const { id, min, max, step } = control;
    console.log(`Checking ${id}${raster ? ' (JPEG)' : ''}`);
    await reveal(id);
    const range = page.locator(`#${id}`), number = page.locator(`#${id}-value`);
    const hasNumber = await number.count() > 0;
    const initial = hasNumber ? Number(await number.inputValue()) : Number(await range.inputValue());
    if (id.startsWith('develop-crop-')) {
      await page.click('#crop-clear');
      if (id.endsWith('-0')) await page.fill('#develop-crop-2-value', '1');
      if (id.endsWith('-1')) await page.fill('#develop-crop-3-value', '1');
    }
    // Both endpoints go through the browser's native range keyboard behavior.
    for (const [key, expected] of [['Home', min], ['End', max]]) {
      await range.focus(); await page.keyboard.press(key);
      close(Number(await range.inputValue()), expected, `${id} ${key}`);
      const displayed = hasNumber ? Number(await number.inputValue()) : expected;
      const recipe = await savedSettings();
      close(valueIn(recipe, id, raster), displayed, `${id} saved ${key}`);
    }
    await range.scrollIntoViewIfNeeded();
    const box = await range.boundingBox();
    await page.mouse.move(box.x + box.width * .3, box.y + box.height / 2); await page.mouse.down();
    await page.mouse.move(box.x + box.width * .65, box.y + box.height / 2, {steps:4}); await page.mouse.up();
    const dragged = hasNumber ? Number(await number.inputValue()) : Number(await range.inputValue());
    const recipe = await savedSettings();
    close(valueIn(recipe, id, raster), dragged, `${id} drag/save`);
    await page.click('#develop-undo'); await page.click('#develop-redo');
    close(hasNumber ? Number(await number.inputValue()) : Number(await range.inputValue()), dragged, `${id} redo`);
    // Reopen the photo through the normal selection boundary, so saved settings rebuild the controls.
    await page.click('#film-next'); await page.click('#film-previous');
    await page.waitForFunction(() => !document.querySelector('#develop-controls').disabled);
    if (id.startsWith('local-') || id.startsWith('mask-')) {
      await page.click('[data-tool="brush"]');
      await page.locator('.mask-layer').first().click();
    }
    await reveal(id);
    close(hasNumber ? Number(await number.inputValue()) : Number(await range.inputValue()), dragged, `${id} reload`);
    if (hasNumber) {
      const numeric = Number((Number(await number.getAttribute('min')) + Number(await number.getAttribute('step')) * 2).toFixed(3));
      await number.fill(String(numeric)); await number.blur();
      close(valueIn(await savedSettings(), id, raster), numeric, `${id} numeric input`);
      await range.dblclick();
      // Crop position prerequisites deliberately differ from default; resetting a position is still zero.
      close(Number(await number.inputValue()), initial, `${id} double-click reset`);
      await savedSettings();
    }
    results.push({id, variant:raster ? 'JPEG WB' : 'RAW/default', min,max,step,passed:true});
  }
  const standard = inventory.filter(c => !c.legacy && c.id.startsWith('develop-'));
  for (const control of standard) await check(control);
  await page.locator('[data-tool=""]').click();
  await page.evaluate(() => { window.rawFixture = false; });
  await page.click('#film-next'); await page.click('#film-previous');
  await page.locator('#raw-status').filter({hasText:'6,504'}).waitFor();
  for (const control of standard.filter(c => c.id.startsWith('develop-raw_'))) await check(control, true);

  await page.click('[data-tool="brush"]');
  const overlay = await page.locator('#develop-overlay').boundingBox();
  await page.mouse.move(overlay.x + overlay.width * .25, overlay.y + overlay.height * .25); await page.mouse.down();
  await page.mouse.move(overlay.x + overlay.width * .7, overlay.y + overlay.height * .7, {steps:5}); await page.mouse.up();
  await page.locator('.mask-layer').waitFor();
  for (const control of inventory.filter(c => c.id.startsWith('local-') || c.id.startsWith('mask-'))) await check(control);
  await page.click('#module-library'); await page.click('#grid-view');
  const tile = page.locator('#tile-size');
  await tile.focus(); await page.keyboard.press('Home');
  const small = await page.locator('.tile').first().boundingBox();
  await page.keyboard.press('End');
  await page.waitForFunction(width => document.querySelector('.tile').getBoundingClientRect().width > width, small.width);
  results.push({id:'tile-size',passed:true});
  assert.equal(results.length, inventory.filter(c=>!c.legacy).length + 2, 'Every accessible slider plus JPEG WB must be covered');
  const report = {browser:process.env.AE_WEBKIT ? 'WebKit' : 'Chromium', controls:inventory.length, legacyHidden:inventory.filter(c=>c.legacy).length, results};
  if (process.env.AE_SLIDER_REPORT) await writeFile(process.env.AE_SLIDER_REPORT, JSON.stringify(report,null,2));
  console.log(`All slider interactions passed: ${results.length} scenarios; ${report.legacyHidden} hidden legacy curve controls excluded.`);
};
