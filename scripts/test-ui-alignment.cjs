const assert = require('node:assert/strict');

module.exports = async function testUiAlignment(page) {
  await page.goto(new URL('/', page.url()).href);
  await page.locator('.film-tile.active-photo').waitFor();
  await page.click('#module-develop');
  await page.locator('#develop-controls:not([disabled])').waitFor();
  await page.locator('#color-grading-panel > summary').click();

  async function centered(selector, square = false) {
    const measurements = await page.locator(selector).evaluateAll((buttons, square) => buttons.map(button => {
      const box = button.getBoundingClientRect();
      const icon = button.querySelector(square ? 'svg circle' : 'svg').getBoundingClientRect();
      return { dx: icon.x + icon.width / 2 - box.x - box.width / 2,
        dy: icon.y + icon.height / 2 - box.y - box.height / 2,
        width: box.width, height: box.height };
    }), square);
    assert.ok(measurements.length > 0, selector);
    for (const box of measurements) {
      assert.ok(Math.abs(box.dx) < .1 && Math.abs(box.dy) < .1,
        `${selector}: icon must share the button center (${box.dx}, ${box.dy})`);
      if (square) assert.ok(Math.abs(box.width - box.height) < .1, `${selector}: circular button must be square`);
    }
  }

  for (const width of [900, 1320, 2048]) {
    await page.setViewportSize({width, height: 1000});
    await centered('.grading-modes button');
    await centered('#curve-channel-buttons button', true);
    await centered('.develop-toolstrip button');
    const wheels = await page.locator('.grading-wheel:visible').evaluateAll(buttons => buttons.map(button => {
      const wheel = button.getBoundingClientRect();
      const handle = button.querySelector('.grading-wheel-handle').getBoundingClientRect();
      return { square: Math.abs(wheel.width - wheel.height),
        dx: handle.x + handle.width / 2 - wheel.x - wheel.width / 2,
        dy: handle.y + handle.height / 2 - wheel.y - wheel.height / 2 };
    }));
    for (const wheel of wheels) {
      assert.ok(wheel.square < .1 && Math.abs(wheel.dx) < .1 && Math.abs(wheel.dy) < .1,
        'neutral grading handle must share the wheel center');
    }
  }
  for (const channel of [0, 1, 2, 3]) {
    await page.click(`#curve-channel-buttons [data-channel="${channel}"]`);
    assert.equal(await page.locator('#develop-curve-channel').inputValue(), String(channel));
    assert.equal(await page.locator(`#curve-channel-buttons [data-channel="${channel}"]`).getAttribute('aria-pressed'), 'true');
  }
  if (process.env.AE_UI_SCREENSHOT) await page.locator('#curve-channel-buttons').screenshot({path: process.env.AE_UI_SCREENSHOT.replace('.png', '-channels.png')});
  await page.click('[data-tool="crop"]');
  for (const width of [900, 1320, 2048]) {
    await page.setViewportSize({width, height: 1000});
    await centered('.crop-icon:has(svg)');
  }
  console.log('UI alignment: concentric channel buttons, grading icons/handles and crop icons at 900/1320/2048px passed.');
};
