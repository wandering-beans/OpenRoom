// Uses the desktop fixture server; no real catalog is changed.
const assert = require('node:assert/strict');
module.exports = async function testRawWhiteBalance(page, url) {
  await page.goto(url);
  await page.locator('#full-image').waitFor({state:'visible'});
  await page.evaluate(() => { window.rawFixture = true; window.holdOriginalLoad = true; });
  await page.click('#module-develop');
  await page.waitForFunction(() => window.releaseOriginalLoad);
  assert.equal(await page.locator('#develop-temperature-value').isVisible(), false, 'RAW must not offer relative WB during decoding');
  await page.evaluate(() => window.releaseOriginalLoad());
  await page.locator('#develop-raw_temperature-value').waitFor({state:'visible'});
  assert.equal(await page.inputValue('#develop-raw_temperature-value'), '4668');
  assert.equal(await page.inputValue('#develop-raw_tint-value'), '22');
  assert.equal(await page.locator('#develop-temperature-value').isVisible(), false);
  const temperature = page.locator('#develop-raw_temperature-value');
  await temperature.fill('5500'); await temperature.dispatchEvent('input'); await temperature.blur();
  const tint = page.locator('#develop-raw_tint-value');
  await tint.fill('-15'); await tint.dispatchEvent('input'); await tint.blur();
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop'
    && c.args.settings.raw_temperature === 5500 && c.args.settings.raw_tint === -15));
  for (const id of ['app-shell', 'workspace']) {
    await page.locator(`#${id}`).evaluate(node => { node.inert = true; });
    await page.keyboard.press('Control+z');
    assert.equal(await tint.inputValue(), '-15', `${id} must block editing shortcuts while busy`);
    await page.locator(`#${id}`).evaluate(node => { node.inert = false; });
  }
  await page.keyboard.press('Control+z');
  assert.equal(await tint.inputValue(), '22');
  await page.keyboard.press('Control+Shift+z');
  assert.equal(await tint.inputValue(), '-15');
  await page.getByRole('button', {name:'撮影時', exact:true}).click();
  assert.equal(await temperature.inputValue(), '4668');
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop'
    && c.args.settings.raw_temperature === null && c.args.settings.raw_tint === 0));
  const travel = await page.locator('#develop-raw_temperature').evaluate(input => ({value:Number(input.value), max:Number(input.max)}));
  assert.ok(travel.value / travel.max > .2 && travel.value / travel.max < .4, 'as-shot WB must have usable slider travel');
  for (const [position, expected] of [[0,2000],[1000,60000],[500,Math.round(Math.sqrt(2000*60000))]]) {
    await page.locator('#develop-raw_temperature').evaluate((input, value) => {
      input.value=value; input.dispatchEvent(new Event('input', {bubbles:true})); input.dispatchEvent(new Event('change', {bubbles:true}));
    },position);
    assert.equal(await temperature.inputValue(),String(expected));
    assert.equal(await page.locator('#develop-raw_temperature').getAttribute('aria-valuetext'),`${expected} K`);
  }
  for (const kelvin of [2000, 60000]) {
    await temperature.fill(String(kelvin)); await temperature.blur();
    await page.waitForFunction(k => calls.some(c => c.command === 'save_develop'
      && c.args.settings.raw_temperature === k), kelvin);
  }
  await page.getByRole('button', {name:'撮影時', exact:true}).click();
  await page.click('#module-library');
  await page.evaluate(async () => {
    const invoke = window.__TAURI__.core.invoke;
    const recipe = await invoke('get_develop', {id:1});
    await invoke('save_develop', {id:1, expectedRevision:recipe.revision, steps:[],
      settings:{...recipe.settings, raw_temperature:null, raw_tint:0, temperature:100, tint:-10}});
  });
  const saveCount = await page.evaluate(() => calls.filter(c => c.command === 'save_develop').length);
  await page.click('#module-develop');
  await page.getByText('旧設定のWB（K値は目安。入力するとK指定に切替）', {exact:true}).waitFor();
  assert.equal(await page.locator('#develop-temperature-value').isVisible(), false);
  assert.equal(await temperature.inputValue(), String(Math.round(4668.35 * Math.SQRT2)));
  assert.equal(await tint.inputValue(), '12');
  assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'save_develop').length), saveCount,
    'opening a legacy recipe must not rewrite it');
  await temperature.fill('4000'); await temperature.blur();
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop'
    && c.args.settings.raw_temperature === 4000 && c.args.settings.raw_tint === 12
    && c.args.settings.temperature === 0 && c.args.settings.tint === 0));
  await page.getByRole('button', {name:'撮影時', exact:true}).click();
  assert.equal(await temperature.inputValue(), '4668');
  assert.equal(await tint.inputValue(), '22');
  await page.goto(url);
  await page.locator('#full-image').waitFor({state:'visible'});
  await page.click('#module-develop');
  await page.getByRole('spinbutton', {name:'色温度（K）の数値', exact:true}).waitFor();
  assert.equal(await temperature.inputValue(), '6504');
  await page.getByText('推定基準 6,504K', {exact:false}).waitFor();
  assert.equal(await page.locator('#develop-temperature-value').count(), 0);
  await temperature.fill('2000'); await temperature.blur();
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop'
    && c.args.settings.raster_temperature === 2000 && c.args.settings.raw_temperature === null
    && c.args.settings.temperature === 0));
  await temperature.fill('60000'); await temperature.blur();
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop'
    && c.args.settings.raster_temperature === 60000));
  await page.getByRole('button', {name:'撮影時', exact:true}).click();
  assert.equal(await temperature.inputValue(), '6504');
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop'
    && c.args.settings.raster_temperature === null && c.args.settings.raster_tint === 0));
  await page.goto(url);
  await page.locator('#full-image').waitFor({state:'visible'});
  await page.evaluate(() => { window.rasterTemperatureFixture = 5200; });
  await page.click('#module-develop');
  await page.getByText('メタデータ取得', {exact:false}).waitFor();
  assert.equal(await temperature.inputValue(), '5200');
  await temperature.fill('7000'); await temperature.blur();
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop'
    && c.args.settings.raster_temperature === 7000 && c.args.settings.raster_reference_temperature === 5200));
  await page.getByRole('button', {name:'撮影時', exact:true}).click();
  assert.equal(await temperature.inputValue(), '5200');
  await page.waitForFunction(() => calls.some(c => c.command === 'save_develop'
    && c.args.settings.raster_temperature === null && c.args.settings.raster_reference_temperature === null));
};
