const assert = require('node:assert/strict');

module.exports = async function testCopySettings(page) {
  await page.goto(new URL('/', page.url()).href);
  await page.waitForFunction(() => document.querySelector('#startup-screen').hidden && document.querySelector('.film-tile'));
  const original = await page.evaluate(async () => {
    const read = id => __TAURI__.core.invoke('get_develop', {id});
    const source = await read(1), destination = await read(2);
    Object.assign(source.settings, {exposure:1.75, saturation:135, distortion:12, angle:15, crop:[.1,.1,.8,.8],
      masks:[{name:'Source mask',enabled:true,invert:false,amount:1,shapes:[],adjustments:{exposure:1}}]});
    Object.assign(destination.settings, {exposure:-.5, angle:-8, crop:[.2,.2,.6,.6], perspective_v:10,
      masks:[{name:'Destination mask',enabled:true,invert:false,amount:1,shapes:[],adjustments:{exposure:-1}}]});
    for (const [id, recipe] of [[1,source],[2,destination]]) {
      await __TAURI__.core.invoke('save_develop', {id,settings:recipe.settings,expectedRevision:recipe.revision,steps:[],undo:[]});
    }
    return {source:source.settings,destination:destination.settings};
  });
  const photo = number => page.locator('.film-tile').filter({has:page.locator('.film-number', {hasText:new RegExp(`^${number}$`)})});
  await photo(1).click();
  await page.keyboard.press('Control+c');
  await page.locator('#copy-settings-dialog[open]').waitFor();
  const selected = await page.locator('[data-copy-field]').evaluateAll(inputs => Object.fromEntries(inputs.map(input => [input.dataset.copyField,input.checked])));
  for (const field of ['monochrome','temperature','exposure','contrast','highlights','shadows','whites','blacks','texture','clarity','dehaze','vibrance','saturation','curves','point_curves','mixer','grading','sharpen','noise_luma','noise_color','distortion','vignette','grain']) assert.equal(selected[field],true,field);
  for (const field of ['perspective_h','angle','crop','masks']) assert.equal(selected[field],false,field);
  await page.getByRole('button',{name:'チェックしない',exact:true}).click();
  assert.equal(await page.locator('#copy-settings-confirm').isEnabled(),false);
  await page.getByRole('button',{name:'すべてをチェック',exact:true}).click();
  assert.equal(await page.locator('[data-copy-field]:not(:checked)').count(),0);
  await page.getByRole('button',{name:'初期設定',exact:true}).click();
  if (process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-copy-settings.png')});
  await page.locator('#copy-settings-confirm').click();
  await photo(2).click();
  await page.keyboard.press('Control+v');
  await page.waitForFunction(async () => (await __TAURI__.core.invoke('get_develop',{id:2})).settings.exposure === 1.75);
  await page.waitForFunction(() => calls.some(c=>c.command==='refresh_library_preview'&&c.args.id===2));
  const pasted = await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop',{id:2})).settings);
  for (const key of Object.keys(pasted)) {
    const retained = ['version','angle','crop','masks','perspective_h','perspective_v','scale','aspect','offset_x','offset_y','constrain_crop','quarter_turns','flip_h','flip_v'].includes(key);
    assert.deepEqual(pasted[key], (retained ? original.destination : original.source)[key], key);
  }
  const save = await page.evaluate(() => calls.filter(c=>c.command==='save_develop'&&c.args.id===2).at(-1).args);
  assert.equal(save.steps[0].label,'設定を貼り付け');
  assert.deepEqual(save.undo.at(-1), original.destination, 'paste preserves the previous state for undo');
  await page.waitForFunction(() => document.querySelector('.film-tile.selected .adjustment-badge'));

  // Cancel keeps the previous clipboard, and text editing never opens the photo dialog.
  await page.keyboard.press('Meta+c');
  await page.locator('#copy-settings-dialog[open]').waitFor();
  await page.locator('[data-copy-field="crop"]').check();
  await page.keyboard.press('Escape');
  await page.waitForFunction(() => !document.querySelector('#copy-settings-dialog').open);
  await page.keyboard.press('Control+f');
  await page.locator('#search').fill('photo');
  await page.keyboard.press('Control+c');
  assert.equal(await page.locator('#copy-settings-dialog').isVisible(),false);
  const savesBeforeTextPaste = await page.evaluate(() => calls.filter(c=>c.command==='save_develop').length);
  await page.locator('#search').evaluate(input => input.dispatchEvent(new KeyboardEvent('keydown',{key:'v',ctrlKey:true,bubbles:true})));
  assert.equal(await page.evaluate(() => calls.filter(c=>c.command==='save_develop').length),savesBeforeTextPaste);
  await page.locator('#search').fill('');
  await page.locator('#search').press('Tab');
  await photo(2).click();

  // The same clipboard works in Develop and uses its undo/save path.
  await page.click('#module-develop');
  await page.waitForFunction(() => !document.querySelector('#develop-controls').disabled);
  await page.locator('#develop-exposure-value').fill('0.25');
  await page.locator('#develop-exposure-value').press('Tab');
  await photo(2).click();
  await page.keyboard.press('Meta+v');
  await page.waitForFunction(() => document.querySelector('#develop-exposure-value').value === '1.75' && document.querySelector('#develop-save').disabled);
  const afterDevelop = await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop',{id:2})).settings);
  assert.deepEqual(afterDevelop.crop,original.destination.crop,'cancelling the second copy retained the original clipboard');
  await page.click('#develop-copy');
  await page.locator('#copy-settings-dialog[open]').waitFor();
  await page.locator('[data-copy-field="exposure"]').uncheck();
  assert.equal(await page.locator('#copy-settings-dialog fieldset').filter({hasText:'基本補正'}).locator('legend input').evaluate(input=>input.indeterminate),true);
  await page.locator('#copy-settings-confirm').click();
  await page.click('#module-library');
  await photo(3).click();
  const untouched = await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop',{id:3})).settings);
  await page.evaluate(() => { window.failDevelopSave = true; });
  await page.keyboard.press('Control+v');
  await page.waitForFunction(() => document.querySelector('#error').textContent.includes('Injected development save failure'));
  assert.deepEqual(await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop',{id:3})).settings), untouched);
  await page.evaluate(() => { window.failDevelopSave = false; });
  await page.keyboard.press('Control+v');
  await page.waitForFunction(async () => (await __TAURI__.core.invoke('get_develop',{id:3})).settings.saturation === 135);
  const custom = await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop',{id:3})).settings);
  assert.equal(custom.exposure,untouched.exposure,'unchecked exposure stays unchanged');
  assert.deepEqual(custom.crop,untouched.crop);
  assert.deepEqual(await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop',{id:1})).settings), original.source,'copy never modifies the source photo');
  console.log('Copy settings: defaults, library/develop paste, Ctrl/Cmd, cancellation, undo, preview and text input guards passed');
};
