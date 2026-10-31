const assert = require('node:assert/strict');

module.exports = async function testSliderScroll(page, url) {
  await page.goto(url);
  await page.click('#module-develop');
  await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-'));
  const panel = page.locator('.develop-scroll');
  const slider = page.locator('#develop-exposure');
  await slider.scrollIntoViewIfNeeded();
  const box = await slider.boundingBox();
  const scrollTop = () => panel.evaluate(node => node.scrollTop);
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  const start = await scrollTop();
  const value = Number(await slider.inputValue());
  await page.mouse.move(box.x + box.width * .7, box.y + box.height / 2, {steps: 5});
  assert.ok(Number(await slider.inputValue()) > value, 'drag must still adjust the value');
  await page.mouse.wheel(0, 150);
  await page.waitForTimeout(200); // Let the browser apply wheel scrolling before inspecting it.
  assert.equal(await scrollTop(), start, 'wheel input during a slider drag must not scroll the panel');
  await page.mouse.up();
  await page.mouse.wheel(0, 150);
  await page.waitForFunction(start => document.querySelector('.develop-scroll').scrollTop > start, start);

  // Cancellation and loss of window focus must never leave scrolling locked.
  for (const end of ['pointercancel', 'lostpointercapture', 'blur']) {
    await slider.scrollIntoViewIfNeeded();
    const bounds = await slider.boundingBox();
    await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
    await page.mouse.down();
    await page.evaluate(end => window.dispatchEvent(end === 'blur'
      ? new Event('blur') : new PointerEvent(end, {pointerId: 1})), end);
    const before = await scrollTop();
    await page.mouse.wheel(0, 120);
    await page.waitForFunction(before => document.querySelector('.develop-scroll').scrollTop > before, before);
    await page.mouse.up();
  }
  await page.locator('#module-develop').focus();
  await slider.evaluate(node => {
    const panel = node.closest('.develop-scroll');
    panel.scrollTop += node.getBoundingClientRect().top - panel.getBoundingClientRect().top + 4;
  });
  const partial = await slider.boundingBox();
  const beforeFocus = await scrollTop();
  await page.mouse.move(partial.x + partial.width / 2, partial.y + partial.height - 2);
  await page.mouse.down();
  assert.equal(await slider.evaluate(node => document.activeElement === node), true);
  assert.equal(await scrollTop(), beforeFocus, 'focusing a partially visible slider must not reposition the panel');
  await page.mouse.move(partial.x - 80, partial.y + partial.height - 2);
  await page.mouse.up();
  await page.mouse.move(partial.x + partial.width / 2, partial.y + partial.height - 2);
  await page.mouse.wheel(0, 80);
  await page.waitForFunction(before => document.querySelector('.develop-scroll').scrollTop > before, beforeFocus);
  await slider.scrollIntoViewIfNeeded();
  await slider.focus();
  await page.keyboard.press('Home');
  const beforeKey = Number(await slider.inputValue());
  await page.keyboard.press('ArrowRight');
  assert.ok(Number(await slider.inputValue()) > beforeKey, 'keyboard adjustment must remain available');
  await page.fill('#develop-exposure-value', '0.25');
  await page.locator('#develop-exposure-value').blur();
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop' && c.args.settings.exposure === .25));
  assert.equal(await slider.evaluate(node => getComputedStyle(node).touchAction), 'none', 'touch dragging must adjust the slider without panning');
  console.log('Slider scroll: drag, wheel suppression/recovery, cancellation, focus loss, keyboard and numeric input passed.');
};
