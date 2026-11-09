const assert = require('node:assert/strict');

module.exports = async function testWarnings(page, url) {
  await page.clock.install();
  await page.goto(url);
  await page.waitForFunction(() => document.querySelector('#startup-screen').hidden &&
    typeof window.listeners['library-preview-failed'] === 'function');
  await page.mouse.move(0, 0);
  const banner = page.locator('#error');
  const close = banner.getByRole('button', {name:'警告を閉じる'});
  const warn = text => page.evaluate(payload => window.listeners['library-preview-failed']({payload}), text);

  await warn('First warning');
  await page.clock.fastForward(5000);
  await warn('Replacement warning');
  await page.clock.fastForward(4000);
  assert.equal(await banner.isVisible(), true);
  assert.equal(await banner.getByRole('alert').textContent(), 'Replacement warning');
  await page.clock.fastForward(4100);
  assert.equal(await banner.isVisible(), false);

  await warn('Read this warning');
  await banner.hover();
  await page.clock.fastForward(9000);
  assert.equal(await banner.isVisible(), true);
  await close.focus();
  await page.mouse.move(0, 0);
  await page.clock.fastForward(9000);
  assert.equal(await banner.isVisible(), true);
  await close.press('Tab');
  await page.clock.fastForward(8100);
  assert.equal(await banner.isVisible(), false);

  await warn('Dismiss me');
  await close.click();
  assert.equal(await banner.isVisible(), false);
  await warn('Dismiss me');
  await close.press('Enter');
  assert.equal(await banner.isVisible(), false);
};
