const assert = require('node:assert/strict');
module.exports = async function testCropPanel(page, url) {
  await page.goto(url);
  await page.click('#module-develop');
  await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-'));
  await page.fill('#develop-exposure-value', '0.5');
  await page.locator('#develop-exposure-value').blur();
  await page.locator('.develop-toolstrip [data-tool="crop"]').click();
  assert.equal(await page.locator('.develop-toolstrip [data-tool="straighten"]').count(), 0);
  assert.equal(await page.locator('#photo-tool-picker').isVisible(), false);
  assert.equal(await page.locator('#crop-advanced').evaluate(node => node.open), false);
  assert.equal(await page.inputValue('#crop-ratio'), 'original');
  assert.equal(await page.getAttribute('#crop-lock', 'aria-pressed'), 'true');
  await page.click('#crop-lock');
  assert.equal(await page.getAttribute('#crop-lock', 'aria-pressed'), 'false');
  await page.selectOption('#crop-ratio', '1');
  assert.equal(await page.getAttribute('#crop-lock', 'aria-pressed'), 'true');
  await page.selectOption('#crop-overlay', 'never');
  assert.equal(await page.locator('[data-crop-grid]').first().isVisible(), false);
  assert.equal(await page.locator('[data-crop-corner]').first().isVisible(), true);
  const angleBar = page.locator('#develop-angle');
  const angleBox = await angleBar.boundingBox();
  await page.mouse.move(angleBox.x + angleBox.width / 2, angleBox.y + angleBox.height / 2);
  await page.mouse.down();
  assert.equal(await page.locator('[data-level-grid]').count(), 2, 'show fine guides even when the normal overlay is off');
  await page.mouse.move(angleBox.x + angleBox.width * .6, angleBox.y + angleBox.height / 2);
  await page.waitForFunction(() => calls.some(c => c.command === 'develop_preview' && c.args.settings.angle !== 0));
  assert.equal(await page.locator('[data-level-grid]').count(), 2, 'render updates must retain the guides while dragging');
  const grid = await page.locator('[data-level-grid]').first().getAttribute('d');
  assert.ok((grid.match(/M/g) || []).length > 10, 'fine grid must have more than thirds');
  assert.match(grid, /v/); assert.match(grid, /h/);
  await page.mouse.up();
  assert.equal(await page.locator('[data-level-grid]').count(), 0);
  assert.equal(await page.locator('[data-crop-grid]').first().isVisible(), false, 'restore the previous overlay preference');
  await angleBar.focus(); await page.keyboard.down('ArrowRight');
  assert.equal(await page.locator('[data-level-grid]').count(), 2);
  await page.keyboard.up('ArrowRight');
  assert.equal(await page.locator('[data-level-grid]').count(), 0);
  await angleBar.dispatchEvent('pointerdown'); await angleBar.dispatchEvent('pointercancel');
  assert.equal(await page.locator('[data-level-grid]').count(), 0);
  await page.selectOption('#crop-overlay', 'auto');
  await page.mouse.move(5, 5);
  assert.equal(await page.locator('[data-crop-grid]').first().isVisible(), false);
  const overlay = await page.locator('#develop-overlay').boundingBox();
  await page.mouse.move(overlay.x + overlay.width / 2, overlay.y + overlay.height / 2);
  assert.equal(await page.locator('[data-crop-grid]').first().isVisible(), true);
  await page.selectOption('#crop-overlay', 'always');
  await page.fill('#develop-angle-value', '3.25'); await page.locator('#develop-angle-value').blur();
  await page.check('#constrain-crop');
  await page.click('#crop-reset');
  assert.equal(await page.inputValue('#develop-angle-value'), '0.00');
  assert.equal(await page.inputValue('#develop-exposure-value'), '0.5', 'tool reset must keep tone adjustments');
  assert.equal(await page.isChecked('#constrain-crop'), false);
  assert.equal(await page.inputValue('#develop-crop-2-value'), '100');
  await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存済み'));
  await page.click('#crop-straighten');
  const level = await page.locator('#develop-overlay').boundingBox();
  await page.mouse.move(level.x + level.width * .2, level.y + level.height * .4);
  await page.mouse.down();
  await page.mouse.move(level.x + level.width * .8, level.y + level.height * .45, {steps:5});
  await page.mouse.up();
  const expectedAngle = -Math.atan2(level.height * .05, level.width * .6) * 180 / Math.PI;
  assert.ok(Math.abs(Number(await page.inputValue('#develop-angle-value')) - expectedAngle) < .1, 'manual level must correct the drawn line');
  assert.equal(await page.getAttribute('#crop-straighten', 'aria-pressed'), 'false', 'manual level must return to crop handles');
  await page.click('#crop-reset');
  await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存済み'));
  // Use the same visible-image path as the native tool, including canvas readback.
  async function showEdge(tilt) {
    await page.evaluate(async tilt => {
      const canvas = document.createElement('canvas'); canvas.width = 320; canvas.height = 240;
      const context = canvas.getContext('2d');
      context.fillStyle = '#fff'; context.fillRect(0, 0, 320, 240);
      if (tilt !== null) {
        context.translate(160, 120); context.rotate(tilt * Math.PI / 180);
        context.fillStyle = '#000'; context.fillRect(-300, -300, 600, 300);
      }
      const image = document.querySelector('#full-image'); image.src = canvas.toDataURL(); await image.decode();
    }, tilt);
  }
  await showEdge(6);
  await page.click('#crop-auto');
  assert.ok(Math.abs(Number(await page.inputValue('#develop-angle-value')) + 6) <= .75, 'auto must correct the detected tilt');
  await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存済み'));
  const angle = await page.inputValue('#develop-angle-value');
  await showEdge(null); await page.click('#crop-auto');
  assert.equal(await page.inputValue('#develop-angle-value'), angle, 'ambiguous images must retain the previous angle');
  assert.match(await page.textContent('#crop-feedback'), /判定できません/);
  await page.click('#crop-straighten');
  assert.equal(await page.getAttribute('#crop-straighten', 'aria-pressed'), 'true');
  await page.click('#crop-straighten');
  await page.click('#crop-reset');
  for (const width of [900, 1320]) {
    await page.setViewportSize({width, height:880});
    const bounds = await page.locator('.crop-panel').boundingBox();
    for (const id of ['crop-lock', 'crop-auto', 'develop-angle-value', 'crop-overlay', 'tool-done']) {
      const rect = await page.locator(`#${id}`).boundingBox();
      assert.ok(rect.x >= bounds.x && rect.x + rect.width <= bounds.x + bounds.width + 1, `${id} must fit at ${width}px`);
    }
    if (process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png', `-crop-panel-${width}.png`)});
  }
  await page.click('#tool-done');
  assert.equal(await page.locator('#develop-overlay').isVisible(), false);
  assert.equal(await page.locator('#rotation-options').isVisible(), false);
  console.log('Unified crop panel: lock, guides, reset isolation, auto/manual angle, close, narrow/wide layouts passed.');
};
