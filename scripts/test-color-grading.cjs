const assert = require('node:assert/strict');

module.exports = async function testColorGrading(page) {
  await page.goto(new URL('/', page.url()).href);
  await page.locator('.film-tile.active-photo').waitFor();
  await page.click('#module-develop');
  await page.locator('#develop-controls:not([disabled])').waitFor();
  await page.locator('#color-grading-panel > summary').click();
  const panel = page.locator('#color-grading-panel');
  await panel.scrollIntoViewIfNeeded();
  assert.equal(await panel.locator('.grading-wheel:visible').count(), 3);
  const boxes = await panel.locator('.grading-zone:visible').evaluateAll(zones => Object.fromEntries(zones.map(zone => {
    const box = zone.getBoundingClientRect(); return [zone.dataset.grading, {x:box.x,y:box.y,width:box.width}];
  })));
  assert.ok(boxes[1].y < boxes[0].y && boxes[1].y < boxes[2].y);
  assert.equal(boxes[0].y, boxes[2].y);
  assert.ok(boxes[0].x < boxes[1].x && boxes[1].x < boxes[2].x);

  async function drag(zone, dx, dy) {
    const wheel = page.locator(`[data-wheel="${zone}"]`);
    await wheel.scrollIntoViewIfNeeded();
    const box = await wheel.boundingBox();
    const x = box.x + box.width / 2, y = box.y + box.height / 2;
    await page.mouse.move(x, y); await page.mouse.down();
    await page.mouse.move(x + box.width / 2 * dx, y + box.height / 2 * dy, {steps:8});
    await page.mouse.up();
  }
  async function values(zone) {
    return Promise.all([0,1,2].map(index => page.locator(`#develop-grading-${zone}-${index}`).inputValue().then(Number)));
  }
  await drag(1, .6, 0);
  assert.deepEqual(await values(1), [0,60,0]);
  await page.click('#develop-undo');
  assert.deepEqual(await values(1), [0,0,0], 'one drag is one undo step');
  await page.click('#develop-redo');
  assert.deepEqual(await values(1), [0,60,0]);
  await drag(0, 0, -.7);
  assert.deepEqual(await values(0), [90,70,0]);
  await drag(2, -2, 0);
  assert.deepEqual(await values(2), [180,100,0], 'capture clamps outside the wheel');
  const selectedPhoto = await page.locator('.film-tile.active-photo').getAttribute('title');
  await page.locator('[data-wheel="2"]').press('Shift+ArrowRight');
  await page.locator('[data-wheel="2"]').press('ArrowDown');
  assert.deepEqual(await values(2), [190,99,0]);
  assert.equal(await page.locator('.film-tile.active-photo').getAttribute('title'), selectedPhoto, 'wheel keys do not navigate photos');

  for (const zone of [0,1,2,3]) {
    await page.click(`[data-grading-mode="${zone}"]`);
    assert.equal(await panel.locator('.grading-wheel:visible').count(), 1);
    assert.equal(await page.locator(`[data-wheel="${zone}"]`).isVisible(), true);
  }
  await drag(3, 0, .5);
  assert.deepEqual(await values(3), [270,50,0]);
  await page.fill('#develop-grading-3-0-value', '220');
  await page.fill('#develop-grading-3-1-value', '25');
  await page.fill('#develop-grading-3-2-value', '-12');
  await page.locator('#develop-grading-3-2-value').blur();
  assert.deepEqual(await values(3), [220,25,-12]);
  await page.fill('#develop-grading_blending-value', '80');
  await page.fill('#develop-grading_balance-value', '-30');
  await page.locator('#develop-grading_balance-value').blur();
  await page.waitForFunction(() => calls.some(call => call.command === 'save_develop' && call.args.settings.grading_balance === -30 && call.args.settings.grading_blending === 80));
  await page.waitForFunction(() => calls.some(call => call.command === 'develop_preview' && call.args.settings.grading[3][0] === 220 && call.args.settings.grading_balance === -30));
  await page.click('#film-next');
  await page.waitForFunction(() => document.querySelector('#develop-grading-3-0').value === '0');
  await page.click('#film-previous');
  await page.waitForFunction(() => document.querySelector('#develop-grading-3-0').value === '220');
  assert.deepEqual(await values(3), [220,25,-12], 'saved grading survives photo navigation');
  assert.equal(await page.locator('#develop-grading_blending').inputValue(), '80');
  assert.equal(await page.locator('#develop-grading_balance').inputValue(), '-30');
  await page.locator('[data-wheel="3"]').dblclick();
  assert.deepEqual(await values(3), [0,0,0]);
  assert.deepEqual(await values(0), [90,70,0], 'zone reset preserves other zones');
  await page.click('[data-grading-mode="-1"]');
  for (const width of [900,1320]) {
    await page.setViewportSize({width,height:900});
    await panel.scrollIntoViewIfNeeded();
    assert.equal(await panel.evaluate(element => element.scrollWidth <= element.clientWidth), true, 'grading fits narrow panels');
    assert.equal(await panel.locator('.grading-wheel:visible').count(), 3);
  }
  if (process.env.AE_UI_SCREENSHOT) await panel.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png', '-grading.png')});
  console.log('Color grading: three-way layout, all zones, drag, keyboard, clamp, numeric input, undo/redo, save/reload and reset passed.');
};
