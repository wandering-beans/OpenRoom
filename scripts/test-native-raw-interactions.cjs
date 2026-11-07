// Real WebView2 and rtengine, against the benchmark's disposable catalog.
const assert = require('node:assert/strict');

module.exports = async function interactions(page, photo, second) {
  await page.locator('#grid-view').click();
  await page.keyboard.press('Control+f');
  await page.locator('#search').fill(photo.path);
  const tile = page.locator('.tile[title='+JSON.stringify(photo.path)+']').first();
  await tile.click({button:'right'});
  const menu = page.locator('#photo-menu');
  await menu.waitFor();
  await page.keyboard.press('Escape');
  for (const [element,key] of [[tile,'ContextMenu'],[page.locator('.folder-row .folder-label').first(),'Shift+F10']]) {
    await element.focus();
    await page.keyboard.press(key);
    await menu.waitFor();
    await page.keyboard.press('Escape');
  }
  // The folder menu may have changed the current root and selection.
  await page.keyboard.press('Control+f');
  await page.locator('#search').fill('');
  await tile.click();
  const inspectorInitiallyHidden = !await page.locator('#library-inspector').isVisible();
  if (inspectorInitiallyHidden) await page.locator('#toggle-inspector').click();
  const fileSummary = page.locator('details:has(#selection-details) > summary');
  const fileInitiallyClosed = await fileSummary.evaluate(el=>!el.parentElement.open);
  if (fileInitiallyClosed) await fileSummary.click();
  const fileInfo = page.locator('#selection-details dd').first();
  await fileInfo.waitFor({state:'attached'});
  const copy = await fileInfo.evaluate(el => {
    const range = document.createRange(); range.selectNodeContents(el);
    const selection = document.getSelection(); selection.removeAllRanges(); selection.addRange(range);
    const allowed = {text:selection.toString(),shortcut:el.dispatchEvent(new KeyboardEvent('keydown',
      {key:'c',ctrlKey:true,bubbles:true,cancelable:true})),
      menu:el.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true}))};
    selection.removeAllRanges();
    return allowed;
  });
  assert(copy.text && copy.shortcut && copy.menu);
  if (fileInitiallyClosed) await fileSummary.click();
  if (inspectorInitiallyHidden) await page.locator('#toggle-inspector').click();
  await page.locator('#loupe-view').click();
  await page.waitForFunction(()=>document.querySelector('#full-image').naturalWidth > 384);
  await page.locator('#navigator-100').click();
  const area = page.locator('#image-area');
  await page.waitForFunction(() => {
    const el = document.querySelector('#image-area');
    return el.scrollWidth > el.clientWidth + 100 && document.querySelector('#navigator-100').classList.contains('active');
  });
  const start = await area.evaluate(el => {
    el.scrollLeft=(el.scrollWidth-el.clientWidth)/2;
    const box=el.getBoundingClientRect(), photo=document.querySelector('#full-image').getBoundingClientRect();
    const x=(Math.max(box.left,photo.left)+Math.min(box.right,photo.right))/2;
    const y=(Math.max(box.top,photo.top)+Math.min(box.bottom,photo.bottom))/2;
    return {x,y,left:el.scrollLeft};
  });
  await page.mouse.move(start.x,start.y);
  await page.mouse.down();
  await page.mouse.move(start.x-40,start.y,{steps:5});
  await page.mouse.up();
  assert(Math.abs(await area.evaluate(el=>el.scrollLeft) - start.left - 40) <= 2,'mouse drag must pan by 40px');
  assert(await page.locator('#navigator-100').evaluate(el=>el.classList.contains('active')),'drag must preserve zoom');
  await page.locator('#navigator-fit').click();
  await page.locator('#module-develop').click();
  await page.waitForFunction(()=>document.querySelector('#develop-histogram').hasAttribute('title'));
  async function drag(x0,y0,x1,y1) {
    const rect = await page.locator('#develop-overlay').boundingBox();
    await page.mouse.move(rect.x+x0*rect.width,rect.y+y0*rect.height);
    await page.mouse.down();
    await page.mouse.move(rect.x+x1*rect.width,rect.y+y1*rect.height,{steps:6});
    await page.mouse.up();
  }
  await page.locator('[data-tool="crop"]').click();
  await page.selectOption('#crop-ratio','0');
  await drag(.2,.2,.7,.7);
  await page.keyboard.press('Enter');
  await page.waitForFunction(async id => (await __TAURI__.core.invoke('get_develop',{id})).settings.crop[2] < .95,photo.id);
  await page.locator('.develop-toolstrip [data-tool="brush"]').click();
  await page.locator('#mask-new').click();
  await page.locator('[data-mask-type="linear"]').click();
  await drag(.35,.25,.35,.65);
  await page.waitForFunction(async id => (await __TAURI__.core.invoke('get_develop',{id})).settings.masks.length > 0,photo.id);
  if (second) {
    for (const target of [second,photo,second]) {
      await page.locator('.film-tile[title='+JSON.stringify(target.path)+']').click();
      await page.waitForFunction(name => document.querySelector('.film-tile.active-photo')?.title === name,target.path);
    }
    await page.waitForFunction(name => document.querySelector('#full-image').alt === name &&
      document.querySelector('#develop-histogram').hasAttribute('title'),second.path);
    const correct = await page.evaluate(async id => {
      const {settings} = await __TAURI__.core.invoke('get_develop',{id});
      const frame = await __TAURI__.core.invoke('develop_preview',{id,settings,bound:1600});
      const reference = new Image();
      reference.crossOrigin = 'anonymous';
      reference.src = frame.inline_image || __TAURI__.core.convertFileSrc(frame.path);
      await reference.decode();
      async function digest(image) {
        const canvas = document.createElement('canvas');
        canvas.width=image.naturalWidth; canvas.height=image.naturalHeight;
        const ctx=canvas.getContext('2d');
        ctx.drawImage(image,0,0);
        return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',
          ctx.getImageData(0,0,canvas.width,canvas.height).data))).join(',');
      }
      return await digest(reference) === await digest(document.querySelector('#full-image'));
    },second.id);
    assert(correct,'The last selected photo must own the displayed pixels');
  }
  await page.locator('#module-library').click();
  await page.waitForFunction(()=>!document.body.classList.contains('develop-mode'));
  console.log(JSON.stringify({interactions:'photo/folder menus, pan, crop, mask, saved recipe and rapid B/A/B pixels',photo:photo.path}));
};
