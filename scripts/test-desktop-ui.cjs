// Test the same Vite output embedded by Tauri. Catalog changes use in-memory fixtures.
const { createServer } = require('node:http');
const { readFile } = require('node:fs/promises');
const path = require('node:path');
const assert = require('node:assert/strict');
const { chromium } = require(require.resolve('playwright', { paths: [path.resolve(__dirname, '../desktop')] }));

async function run() {
  const root = path.resolve(process.env.AE_UI_ROOT || path.resolve(__dirname, '../desktop/ui-dist'));
  await readFile(path.join(root, 'index.html'));
  const server = createServer(async (request, response) => {
    const name = new URL(request.url, 'http://localhost').pathname.slice(1) || 'index.html';
    if (name === 'test-photo.png' && process.env.AE_UI_PHOTO) {
      response.setHeader('Content-Type', 'image/png');
      response.end(await readFile(process.env.AE_UI_PHOTO));
      return;
    }
    const file = path.resolve(root, name);
    if (!file.startsWith(root + path.sep)) { response.writeHead(403).end(); return; }
    try {
      const bytes = await readFile(file);
      response.setHeader('Content-Type', name.endsWith('.png') ? 'image/png' : name.endsWith('.js') ? 'text/javascript' : name.endsWith('.css') ? 'text/css' : 'text/html');
      response.end(bytes);
    } catch { response.writeHead(404).end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch({ headless: true, executablePath: process.env.AE_CHROMIUM_EXECUTABLE });
    const page = await browser.newPage({ viewport: { width: 2048, height: 1012 } });
    let discardResponse;
    const decideDiscard = accept => {
      discardResponse = (async () => {
        await page.locator('#discard-dialog[open]').waitFor();
        await page.locator(`#discard-dialog button[value="${accept ? 'discard' : 'cancel'}"]`).click();
        await page.waitForFunction(() => !document.querySelector('#discard-dialog').open);
      })();
    };
    const failures = [];
    page.on('pageerror', error => {failures.push(error.message);console.error(error.stack);});
    const photoSize = process.env.AE_UI_PHOTO ? await page.evaluate(url => new Promise((resolve, reject) => {
      const image = new Image();
      image.onload = () => resolve({width:image.naturalWidth, height:image.naturalHeight});
      image.onerror = () => reject(Error('Cannot load AE_UI_PHOTO fixture'));
      image.src = url;
    }), `http://127.0.0.1:${server.address().port}/test-photo.png`) : null;
    await page.addInitScript(({usePhoto, developEndpoint, saveBenchmark}) => {
      const previewSize = (id, bound) => {
        const size = usePhoto || (id % 2 ? {width:2,height:3} : {width:3,height:2});
        const scale = bound / Math.max(size.width,size.height);
        return {width:Math.round(size.width*scale), height:Math.round(size.height*scale)};
      };
      const formats = ['jpg', 'jpeg', 'psd', 'psb', 'tif', 'tiff', 'raf', 'cr2'];
      const items = Array.from({ length: 2000 }, (_, index) => {
        const folder = index < 400 ? '20261019撮影/選定' : index < 1200 ? '20261019撮影' : '20261020撮影';
        const name = `photo-${String(index).padStart(4, '0')}.${formats[index % 8]}`;
        return { id: index + 1, root_id: 1, path: `${folder}/${name}`, name, extension: formats[index % 8],
          size: 1048576, modified: 1700000000000000000, added: 1700000000000, missing: false, rating: 0 };
      });
      const recipes = new Map(), histories = new Map(), libraryRevisions = new Map();
      let activePreviewId = null;
      let nextHistoryId = 1;
      const savedRecipes=[];
      const defaults = {
        version: 1, masks:[], aspect:0, offset_x:0, offset_y:0, constrain_crop:false, exposure: 0, temperature: 0, tint: 0, raw_temperature: null, raw_tint: 0, raster_temperature: null, raster_reference_temperature: null, raster_tint: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0,
        texture: 0, clarity: 0, dehaze: 0, vibrance: 0, saturation: 0, monochrome: false,
        point_curves:[[],[],[],[]], curves: Array.from({length:4}, () => [0,.25,.5,.75,1]), mixer: Array.from({length:8}, () => [0,0,0]),
        grading: Array.from({length:4}, () => [0,0,0]), grading_balance:0, grading_blending:50,
        sharpen:0, sharpen_radius:1, sharpen_detail:25, sharpen_masking:0, noise_luma:0, noise_color:0, ca_auto:false, ca_red:0, ca_blue:0, defringe:false, defringe_radius:2, defringe_threshold:13,
        crop:[0,0,1,1],quarter_turns:0,flip_h:false,flip_v:false,angle:0,perspective_h:0,perspective_v:0,scale:100,distortion:0,
        vignette:0,vignette_midpoint:50,vignette_feather:50,grain:0,grain_size:25,grain_roughness:50,
      };
      recipes.set(5, {settings:{...structuredClone(defaults), exposure:0.5}, revision:1});
      const adjusted = id => {
        const settings = recipes.get(id)?.settings;
        return settings && Object.keys(defaults).some(key => JSON.stringify(settings[key]) !== JSON.stringify(defaults[key]));
      };
      const annotations = new Map();
      let photoVersion = 0;
      const mcpRequests = new Map(), mcpReplies = new Map();
      window.expiredMcp = new Set();
      let mcpSequence = 0;
      let mcpStatus = {enabled:false,running:false,port:39127,url:'http://127.0.0.1:39127/mcp',error:null};
      window.dispatchMcp = (name, arguments_ = {}) => new Promise(resolve => {
        const id = `test-ui-${++mcpSequence}`;
        const request = {id, name, arguments:arguments_};
        mcpRequests.set(id, request); mcpReplies.set(id, resolve);
        window.latestMcpId = id;
        window.listeners['mcp-request']({payload:request});
      });
      window.calls = [];
      window.catalogGeneration = 1;
      window.listeners = {};
      window.slow = false;
      window.realPreviewPending = 0;
      const rejectedPreviews = new Set(), jpegPreviews = new Set();
      const filter = query => {
        if (query.folder != null && query.root_id == null) throw Error('folder filtering requires a root');
        return items.filter(item =>
        (!query.ids?.length || query.ids.includes(item.id)) &&
        (query.root_id == null || item.root_id === query.root_id) &&
        (!query.extensions.length || query.extensions.includes(item.extension)) &&
        (!query.folder || item.path.startsWith(query.folder + '/')) &&
        (!query.flags?.length || query.flags.includes(item.flag || 0)) &&
        (!query.labels?.length || query.labels.includes(item.label || '')) &&
        (query.rating === null || item.rating === query.rating) &&
        (!query.text || [item.name, ...(annotations.get(item.id)?.tags || []), annotations.get(item.id)?.memo || ''].join(' ').includes(query.text)));
      };
      window.__TAURI__ = {
        core: {
          convertFileSrc: preview => {
            if (window.failImageOnce && preview === `preview-${window.failImageOnce}-384`) {
              window.failImageOnce = false; window.failedPreview = preview; rejectedPreviews.add(preview);
            }
            if (rejectedPreviews.has(preview)) return '/missing-preview.png';
            const match = /^preview-(\d+)-(\d+)/.exec(preview);
            const portrait = !match || Number(match[1]) % 2 === 1;
            const bound = match ? Number(match[2]) : 720;
            const width = portrait ? Math.round(bound * 2 / 3) : bound;
            const height = portrait ? bound : Math.round(bound * 2 / 3);
            return usePhoto ? `/test-photo.png?${preview}` : 'data:image/svg+xml,' + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 480 720"><rect width="480" height="720" fill="#8995a0"/><path d="M0 650L170 280L480 650V720H0" fill="#354439"/></svg>`) + `#${preview}`;
          },
          invoke: async (command, args = {}) => {
            const call = { command, args, start: performance.now() };
            window.calls.push(call);
            if (command === 'app_version') return '0.7.9';
            if (command === 'check_app_update') {
              if (window.failUpdateCheck) throw Error('Injected network failure');
              return window.availableUpdate || null;
            }
            if (command === 'install_app_update') {
              if (window.holdUpdate) await new Promise(resolve => { window.releaseUpdate = resolve; });
              if (window.failUpdateInstall) throw Error('Injected signature failure');
              return;
            }
            if (saveBenchmark && ['save_develop','get_develop'].includes(command)) {
              const result = await fetch(developEndpoint + (command === 'save_develop' ? '/save' : '/get'),
                {method:'POST',headers:{'Content-Type':'text/plain'},body:JSON.stringify(args)}).then(response=>response.json());
              call.end = performance.now();
              if (result.error) throw Error(result.error);
              return result;
            }
            if (command === 'develop_preview' && developEndpoint) {
              window.realPreviewPending++;
              try {
                const result = await fetch(developEndpoint, {method:'POST',headers:{'Content-Type':'text/plain'},body:JSON.stringify(args)}).then(response => response.json());
                if (result.error) throw Error(result.error);
                call.end = performance.now(); call.render_ms = result.info.elapsed_ms;
                window.realRenderedExposure = args.settings.exposure;
                return result;
              } finally { window.realPreviewPending--; }
            }
            if (command === 'get_language') return localStorage.getItem('test-language') || 'ja';
            if (command === 'set_language') {
              if (window.failLanguage) throw Error('Injected language save failure');
              localStorage.setItem('test-language-saves', JSON.stringify(calls.filter(call=>call.command==='save_develop')));
              localStorage.setItem('test-language', args.language); return;
            }
            if (command === 'mcp_ready' || command === 'mcp_status') return {...mcpStatus};
            if (command === 'mcp_configure') return mcpStatus = {...mcpStatus, ...args, running:args.enabled, url:`http://127.0.0.1:${args.port}/mcp`};
            if (command === 'mcp_token') return 'test-only-ui-token';
            if (command === 'mcp_rotate') return {...mcpStatus};
            if (command === 'mcp_claim') {
              if (window.expiredMcp.has(args.id)) throw Error('UI request expired');
              return mcpRequests.get(args.id);
            }
            if (command === 'mcp_execute') {
              if (window.expiredMcp.has(args.id)) throw Error('UI request expired');
              const request = mcpRequests.get(args.id), input = request.arguments;
              if (request.name === 'update_photo_attributes') {
                await window.__TAURI__.core.invoke('set_photo_attributes', {rating:null,flag:null,label:null,...input});
                return {updated_ids:input.ids};
              }
              if (request.name === 'update_develop_settings') {
                const saved = recipes.get(input.id) || {settings:defaults,revision:0};
                const settings = {...saved.settings,...input.patch};
                const revision = await window.__TAURI__.core.invoke('save_develop', {id:input.id,settings,expectedRevision:input.expected_revision,steps:[{settings,label:'AIによる現像'}]});
                return {updated_ids:[input.id],revision,settings};
              }
              return {catalog:'/catalogs/catalog.db',jobs:[]};
            }
            if (command === 'mcp_complete') { mcpReplies.get(args.id)({value:args.value,error:args.error}); return; }
            if (command === 'roots') return [{ id: 1, path: '/photo-library' }, ...(window.addedRoots || [])].filter(root => !window.removedRootIds?.includes(root.id));
            if (command === 'remove_from_catalog') {
              if (window.failRemoval) throw Error('Injected catalog removal failure');
              const selection = args.selection;
              const matches = item => selection.kind === 'photos' ? selection.ids.includes(item.id) : item.root_id === selection.root_id && (!selection.path || item.path.startsWith(selection.path + '/'));
              const removed = items.filter(matches);
              for (const item of removed) {
                items.splice(items.indexOf(item), 1);
                recipes.delete(item.id); histories.delete(item.id); annotations.delete(item.id); libraryRevisions.delete(item.id);
              }
              if (selection.kind === 'folder' && !selection.path) (window.removedRootIds ||= []).push(selection.root_id);
              window.catalogGeneration++;
              return removed.length;
            }
            if (command === 'choose_folder') {
              const selected = window.folderChoices ?? [{id:2,path:'/new-photos'}];
              window.addedRoots = selected;
              return selected;
            }
            if (command === 'prepare_library_previews') {
              window.listeners['preview-progress']?.({payload:{total:items.length,ready:items.length,failed:0}});
              return {total:items.length,ready:items.length,generated:items.length,failed:0};
            }
            if (command === 'scan') {
              if (window.holdScan) await new Promise(resolve => {window.releaseScan = resolve;});
              const existing = items.filter(item => args.rootId == null || item.root_id === args.rootId).length;
              if (args.rootId >= 2 && !items.some(item => item.root_id === args.rootId)) {
                for (let i = 0; i < 42; i++) items.push({...items[0],id:args.rootId*3000+i,root_id:args.rootId,path:`IMGP${i}.JPG`,name:`IMGP${i}.JPG`,extension:'jpg',rating:0});
              }
              window.listeners['preview-progress']?.({payload:{total:42,ready:42,failed:0}});
              return {indexed:existing ? 0 : 42,unchanged:existing,missing:0,visited:existing || 42,errors:[],cancelled:Boolean(window.cancelledScan),previews:{total:existing || 42,ready:existing || 42,generated:existing ? 0 : 42,failed:0}};
            }
            if (command === 'cancel_scan') {window.cancelledScan = true;window.releaseScan?.();return;}
            if (command === 'catalog_needs_creation') return new URL(location.href).searchParams.get('startup') === 'new';
            if (command === 'catalog_path') {
              const startupMode = new URL(location.href).searchParams.get('startup');
              if (startupMode === 'hold') await new Promise(resolve => { window.releaseStartup = resolve; });
              if (startupMode === 'fail') throw Error('Injected startup catalog failure');
              return '/catalogs/catalog.db';
            }
            if (command === 'save_catalog_copy') return '/backups/catalog-copy.openroom';
            if (command === 'choose_catalog') {
              if (window.failCatalog) throw Error('Injected invalid catalog');
              return window.catalogChoice || null;
            }
            if (command === 'restart_catalog') {
              if (window.failRestart) throw Error('Injected restart failure');
              return;
            }
            if(command === 'develop_workflow') {
              const request=args.request;
              if(request.action==='list')return savedRecipes.filter(row=>row.item_id===request.item_id);
              if(request.action==='store'){const patch=request.item_id?request.settings:Object.fromEntries(request.fields.map(key=>[key,request.settings[key]]));const row={id:savedRecipes.length+1,item_id:request.item_id,name:request.name,recipe:JSON.stringify(patch)};savedRecipes.push(row);return row.id;}
              if(request.action==='rename'){savedRecipes.find(row=>row.id===request.id).name=request.name;return;}
              if(request.action==='delete'){savedRecipes.splice(savedRecipes.findIndex(row=>row.id===request.id),1);return;}
              if(request.action==='rename_history'){histories.get(request.item_id).find(row=>row.id===request.id).label=request.name;return;}
              if(request.action==='apply'){for(const id of request.ids){const old=recipes.get(id)||{settings:structuredClone(defaults),revision:0};for(const field of request.fields)old.settings[field]=structuredClone(request.settings[field]);old.revision++;recipes.set(id,old);}return request.ids.length;}
            }
            if(command==='choose_export_folder') {
              if (!document.getElementById('export-browse').disabled) {
                throw new Error('Commit UI changes before opening the native folder sheet');
              }
              return window.cancelExportFolder ? null : '/exports';
            }
            if(command==='export_develop_batch') {
              const {ids, folder} = args.request;
              if(window.failBatchExport)throw Error('export failed');
              if(window.holdBatchExport)return new Promise(resolve => { window.releaseBatchExport = () => resolve({completed:0,total:ids.length,cancelled:true,errors:[],folder}); });
              return {completed:ids.length,total:ids.length,cancelled:false,errors:[],folder};
            }
            if(command==='cancel_batch_export'){window.releaseBatchExport?.();return;}
            if (command === 'develop_history') return (histories.get(args.id) || []).filter(row => args.before == null || row.id < args.before).slice().reverse().slice(0, 100).map(({id,label,created}) => ({id,label,created}));
            if (command === 'history_settings') return structuredClone(histories.get(args.id).find(row => row.id === args.historyId).settings);
            if (command === 'folders') return [
              { root_id: 1, path: '', count: 2000 }, { root_id: 1, path: '20261019撮影', count: 1200 },
              { root_id: 1, path: '20261019撮影/選定', count: 400 }, { root_id: 1, path: '20261020撮影', count: 800 },
              ...(window.addedRoots || []).map(root => ({root_id:root.id,path:'',count:items.filter(item=>item.root_id===root.id).length})),
            ].filter(folder => !window.removedRootIds?.includes(folder.root_id)).map(folder => ({...folder, count:items.filter(item => item.root_id === folder.root_id && (!folder.path || item.path.startsWith(folder.path + '/'))).length})).filter(folder => !folder.path || folder.count);
            if (command === 'count') return filter(args.query).length;
            if (command === 'search') {
              if (args.cursor && args.cursor.generation !== window.catalogGeneration) {
                throw Error('stale or incompatible cursor; restart pagination');
              }
              const all = filter(args.query), offset = args.cursor?.offset || 0;
              return { items: all.slice(offset, offset + 200).map(item => ({ ...item })),
                adjusted_ids: all.slice(offset, offset + 200).filter(item => adjusted(item.id)).map(item => item.id),
                next: offset + 200 < all.length ? { offset: offset + 200, generation: window.catalogGeneration } : null,
                generation: window.catalogGeneration };
            }
            if (command === 'detail') {
              const item = items.find(item => item.id === args.id);
              return { item: { ...item }, tags: annotations.get(item.id)?.tags || [], memo: annotations.get(item.id)?.memo || '' };
            }
            if (command === 'index_metadata_batch') {
              if (window.holdMetadataIndex) await new Promise(resolve => { window.releaseMetadataIndex = resolve; });
              return {processed:0,failed:0,remaining:false};
            }
            if (command === 'cancel_metadata_index') {
              window.holdMetadataIndex = false;
              window.releaseMetadataIndex?.();
              return;
            }
            if (command === 'metadata_facets') {
              if (args.field === 'extension') {
                const counts = new Map();
                for (const item of filter({...args.query, extensions:[]})) counts.set(item.extension, (counts.get(item.extension) || 0) + 1);
                return [...counts].map(([value,count]) => ({value,count}));
              }
              return [{value:args.field==='camera'?'Canon EOS M5':args.field==='date'?'2026-10-20':'',count:53}];
            }
            if (command === 'set_photo_attributes') {for(const id of args.ids){const item=items.find(i=>i.id===id);for(const key of ['rating','flag','label'])if(args[key]!==null)item[key]=args[key];}return;}
            if (command === 'annotate') {
              if (window.failSave) throw Error('Injected save failure');
              for (const id of args.ids) { annotations.set(id, args.annotation); items.find(item => item.id === id).rating = args.annotation.rating; }
              return;
            }
            if (command === 'photo_metadata') {
              const version = ++photoVersion;
              await new Promise(resolve => setTimeout(resolve, window.slow ? 150 : args.id === 1 ? 90 : 10));
              if (version !== photoVersion) throw Error('Obsolete metadata');
              return { 'カメラ': `Camera ${args.id}`, 'ISO': '400', '絞り': 'f/2.8', ...(args.id === 1 ? {'焦点距離':'24 mm', 'シャッター速度':'1/250 秒'} : {}) };
            }
            if (command === 'preview') {
              if (args.large && window.failReviewPreviewIds?.includes(args.id)) throw Error('Injected review preview failure');
              if (!args.large && args.id === window.failThumbnailOnce) {
                window.failThumbnailOnce = false; window.failedThumbnailId = args.id;
                throw Error('Injected temporary thumbnail failure');
              }
              const revision = libraryRevisions.get(args.id) || 0;
              const size = previewSize(args.id, args.bound || 384);
              if (window.holdLargePreview && args.large) await new Promise(resolve => { window.releaseLargePreview = resolve; });
              if (window.holdThumbnail && args.id === 1 && !args.large) {
                window.holdThumbnail = false;
                await new Promise(resolve => { window.releaseThumbnail = resolve; });
              }
              await new Promise(resolve => setTimeout(resolve, window.slow ? 150 : 8));
              if (window.inlineLibraryPreview && args.large) {
                const canvas = document.createElement('canvas');
                canvas.width = size.width; canvas.height = size.height;
                canvas.getContext('2d').fillRect(0,0,size.width,size.height);
                return {path:'',inline_image:canvas.toDataURL('image/png'),width:4000,height:6000,
                  render_width:size.width,render_height:size.height};
              }
              return { path: `preview-${args.id}-${args.bound || 384}${revision ? `-r${revision}` : ''}${jpegPreviews.has(args.id) ? '-jpeg' : ''}`, width: 4000, height: 6000,
                render_width:size.width, render_height:size.height };
            }
            if (command === 'get_develop') return { undo: structuredClone((histories.get(args.id) || []).slice(0,-1).slice(-10).map(row=>row.settings)), settings: structuredClone(recipes.get(args.id)?.settings || defaults), revision: recipes.get(args.id)?.revision || 0, defaults: structuredClone(defaults) };
            if (command === 'save_develop') {
              if (window.failDevelopSave) throw Error('Injected development save failure');
              if (window.holdDevelopSave) {
                window.holdDevelopSave = false;
                await new Promise(resolve => { window.releaseDevelopSave = resolve; });
              }
              const previousRevision = recipes.get(args.id)?.revision || 0;
              if (args.expectedRevision !== previousRevision) throw Error('Stale recipe revision');
              const revision = previousRevision + 1;
              const history = args.undo ? args.undo.map(settings=>({id:nextHistoryId++,label:'現像設定',created:Date.now(),settings:structuredClone(settings)})) : histories.get(args.id) || [{id: nextHistoryId++, label:'編集開始時の設定', created:Date.now(), settings:structuredClone(defaults)}];
              for (const step of args.steps) history.push({id:nextHistoryId++, label:step.label, created:Date.now(), settings:structuredClone(step.settings)});
              histories.set(args.id, history.slice(-11));
              recipes.set(args.id, { settings: structuredClone(args.settings), revision }); return revision;
            }
            if (command === 'develop_preview') {
              if (window.failDevelopPreview) throw Error('Injected preview failure');
              if (window.holdDevelopPreview) await new Promise(resolve => {
                window.releaseDevelopPreview = () => { window.releaseDevelopPreview = null; resolve(); };
              });
              const held = window.holdOriginalLoad;
              if (held) { window.holdOriginalLoad = false; await new Promise(resolve => {window.releaseOriginalLoad = resolve;}); }
              await new Promise(resolve => setTimeout(resolve, window.previewDelay ?? (args.settings.exposure === 1 ? 200 : 20)));
              const path = `develop-${args.id}-${args.settings.exposure}-${args.bound}${held ? "-stale" : ""}`;
              const inline_image = [384, 1600].includes(args.bound) ? window.__TAURI__.core.convertFileSrc(path) : undefined;
              if (held) window.originalLoadFinished = true;
              return { path: inline_image ? '' : path, inline_image, info: {...previewSize(1,args.bound === 384 ? 384 : 720),elapsed_ms:25,histogram:window.histogramFixture ?? Array.from({length:3},()=>Array(256).fill(100))}, base:{auto_ca_supported:window.autoCaFixture === true,source_width:4000,source_height:6000,raster_wb_temperature:window.rasterTemperatureFixture ?? null,color_note:window.rawFixture ? 'Canon EOS 350D / Adobe Standard' : 'テスト用の線形sRGB',raw:window.rawFixture ? {as_shot:[4668.35,22.3]} : null} };
            }
            if (command === 'export_develop') return '/exports/edited.jpg';
            if (command === 'invalidate_library_preview') { jpegPreviews.add(args.id); return; }
            if (command === 'refresh_library_preview') {
              if (window.failLibraryRefresh) throw Error('Injected library refresh failure');
              libraryRevisions.set(args.id, recipes.get(args.id)?.revision || 0);
              return;
            }
            if (command === 'set_preview_photo') {
              if(activePreviewId!==args.id) {
                activePreviewId=args.id;
                for(const [id, recipe] of recipes) if(id!==args.id && libraryRevisions.get(id)!==recipe.revision) {
                  libraryRevisions.set(id,recipe.revision);
                  window.listeners['library-preview-updated']?.({payload:id});
                }
              }
              return;
            }
            if (command === 'set_develop_prefetch') return;
            if (command === 'arm_close_guard' || command === 'close_window' || command === 'cancel_preview' || command === 'cancel_develop_preview' || command === 'reveal' || command === 'reveal_catalog') return;
            throw Error('Unexpected command ' + command);
          },
        },
        event: { listen: async (name, callback) => { window.listeners[name] = callback; return () => {}; } },
      };
    }, {usePhoto:photoSize, developEndpoint:process.env.AE_DEVELOP_ENDPOINT, saveBenchmark: Boolean(process.env.AE_SAVE_BENCH)});
    if (process.argv.includes('--updates')) {
      await require('./test-updates-ui.cjs')(page, `http://127.0.0.1:${server.address().port}`);
      assert.deepEqual(failures, []); return;
    }
    if (process.env.AE_UI_LANGUAGE_ONLY || process.argv.includes('--language')) {
      await require('./test-language-ui.cjs')(page, `http://127.0.0.1:${server.address().port}`);
      assert.deepEqual(failures, []); return;
    }
    await page.goto(`http://127.0.0.1:${server.address().port}?startup=new`);
    await page.locator('#catalog-setup').waitFor({state:'visible'});
    await page.setViewportSize({width:900,height:600});
    const setupLayout = await page.evaluate(() => {
      const brand = document.querySelector('.startup-identity').getBoundingClientRect();
      const setup = document.querySelector('#catalog-setup').getBoundingClientRect();
      return {brandBottom:brand.bottom, setupTop:setup.top, setupBottom:setup.bottom, height:innerHeight};
    });
    assert.ok(setupLayout.brandBottom < setupLayout.setupTop && setupLayout.setupBottom <= setupLayout.height, 'catalog setup fits without overlapping at 900x600');
    await page.setViewportSize({width:2048,height:1012});
    assert.equal(await page.locator('#app-shell').evaluate(shell => shell.inert), true);
    assert.equal(await page.evaluate(() => calls.some(c => ['catalog_path','roots','scan','mcp_ready'].includes(c.command))), false, 'first launch must wait for a named catalog');
    await page.click('#catalog-setup-new');
    await page.waitForFunction(() => !document.querySelector('#catalog-setup-new').disabled);
    assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'choose_catalog').at(-1).args.create), true);
    assert.equal(await page.evaluate(() => calls.some(c => c.command === 'restart_catalog')), false, 'cancelled name dialog keeps setup open');
    await page.evaluate(() => window.failCatalog = true);
    await page.click('#catalog-setup-new');
    await page.waitForFunction(() => document.querySelector('#catalog-setup-error').textContent.includes('Injected'));
    assert.equal(await page.locator('#app-shell').evaluate(shell => shell.inert), true);
    await page.evaluate(() => {window.failCatalog = false;window.catalogChoice = '/catalogs/My Photos.openroom';});
    await page.click('#catalog-setup-new');
    await page.waitForFunction(() => calls.some(c => c.command === 'restart_catalog'));
    assert.equal(await page.locator('#app-shell').evaluate(shell => shell.inert), true, 'wait for restart before loading the named catalog');
    await page.goto(`http://127.0.0.1:${server.address().port}?startup=new`);
    await page.locator('#catalog-setup').waitFor({state:'visible'});
    await page.evaluate(() => window.catalogChoice = '/catalogs/Existing.openroom');
    await page.click('#catalog-setup-open');
    await page.waitForFunction(() => calls.some(c => c.command === 'restart_catalog'));
    assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'choose_catalog').at(-1).args.create), false);
    await page.goto(`http://127.0.0.1:${server.address().port}?startup=hold`);
    await page.waitForFunction(() => window.releaseStartup);
    assert.equal(await page.title(), 'OpenRoom — ライブラリ');
    assert.equal(await page.locator('#startup-screen').isVisible(), true);
    assert.equal(await page.locator('#app-shell').evaluate(shell => shell.inert), true);
    const startupCalls = await page.evaluate(() => calls.length);
    await page.keyboard.press('d');
    assert.equal(await page.evaluate(() => calls.length), startupCalls, 'startup blocks module shortcuts');
    await page.waitForFunction(() => [...document.querySelectorAll('img[src^="branding/"]')].every(image => image.complete && image.naturalWidth > 0));
    if (process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png', '-startup.png')});
    await page.evaluate(() => window.releaseStartup());
    await page.locator('#startup-screen').waitFor({state:'hidden'});
    assert.equal(await page.locator('#app-shell').evaluate(shell => shell.inert), false);
    assert.equal(await page.locator('.identity small').textContent(), 'OpenRoom');
    await page.waitForFunction(() => document.querySelector('#view-message').hidden && document.querySelector('#full-image').src.includes('preview-1-1600'));
    assert.equal(await page.locator('#viewport').isVisible(), false);
    assert.equal(await page.locator('#library-inspector').isVisible(), false);
    assert.equal(await page.locator('#filter-panel').isVisible(), true);
    async function checkAdjustmentBadge(selector) {
      await page.waitForFunction(selector => {
        const tile = document.querySelector(selector), image = tile?.querySelector('img'), badge = tile?.querySelector('.adjustment-badge');
        if (!image?.complete || !image.naturalWidth || !badge) return false;
        const photo = image.getBoundingClientRect(), icon = badge.getBoundingClientRect();
        return icon.left >= photo.left && icon.right <= photo.right && icon.top >= photo.top && icon.bottom <= photo.bottom;
      }, selector);
    }
    assert.ok(await page.locator('.film-tile').count() < 35, 'filmstrip must virtualize');
    assert.equal(await page.locator('.film-tile.selected .adjustment-badge').count(), 0, 'unadjusted photos have no badge');
    assert.equal(await page.locator('.film-tile[title="photo-0004.tif"] .adjustment-badge').count(), 1, 'saved adjustments are marked on catalog load');
    assert.ok(await page.evaluate(() => !calls.some(call => call.command === 'preview' && call.args.bound === 4096)), 'first view should use 1600px');
    assert.equal(await page.locator('.module:disabled').count(), 0);
    await page.waitForFunction(() => !document.querySelector('#navigator-image').hidden && [...document.querySelectorAll('.film-tile')].every(tile => {
      const image = tile.querySelector('img');
      return image?.complete && image.naturalWidth > 0;
    }));
    await checkAdjustmentBadge('.film-tile[title="photo-0004.tif"]');
    if (process.env.AE_UI_SCREENSHOT) await page.screenshot({ path: process.env.AE_UI_SCREENSHOT });
    await page.evaluate(() => window.listeners['preview-progress']({payload:{total:0,ready:0,failed:0}}));
    assert.equal(await page.locator('#progress').isVisible(), false, 'no startup work must not show a progress banner');
    await page.evaluate(() => window.listeners['preview-progress']({payload:{total:2,ready:1,failed:0}}));
    assert.equal(await page.locator('#progress').isVisible(), true);
    assert.match(await page.locator('#progress-text').textContent(), /1\/2/, 'progress counts only pending photos');
    await page.evaluate(() => {document.getElementById('progress').hidden = true;});
    if (process.argv.includes('--startup-previews')) { assert.deepEqual(failures, []); return; }
    if (process.argv.includes('--crop-pan')) { await require('./test-crop-pan.cjs')(page, `http://127.0.0.1:${server.address().port}`); assert.deepEqual(failures, []); return; }
    if (process.argv.includes('--crop-panel')) { await require('./test-crop-panel.cjs')(page, `http://127.0.0.1:${server.address().port}`); assert.deepEqual(failures, []); return; }
    if (process.argv.includes('--interactive-preview')) { await require('./test-interactive-preview.cjs')(page); assert.deepEqual(failures, []); return; }
    if (process.argv.includes('--copy-settings')) { await require('./test-copy-settings.cjs')(page); assert.deepEqual(failures, []); return; }
    if (process.argv.includes('--lens')) { await require('./test-lens-correction.cjs')(page); assert.deepEqual(failures, []); return; }
    if (process.argv.includes('--grading')) { await require('./test-color-grading.cjs')(page); assert.deepEqual(failures, []); return; }
    if (process.argv.includes('--slider-benchmark')) { await require('./benchmark-slider-controls.cjs')(page); assert.deepEqual(failures, []); return; }
    if (process.env.AE_DEVELOP_ENDPOINT) {
      await require(process.env.AE_SAVE_BENCH ? './benchmark-autosave-ui.cjs' : './benchmark-develop-ui.cjs').measure(page);
      return;
    }
    const initialLayout = await page.evaluate(() => {
      const rect = id => { const box = document.getElementById(id).getBoundingClientRect(); return { x: box.x, y: box.y, width: box.width, height: box.height }; };
      return { header: rect('module-bar'), sidebar: rect('sidebar'), viewer: rect('viewer'), filmstrip: rect('filmstrip'), image: rect('full-image') };
    });
    assert.equal(initialLayout.header.height, 64);
    assert.ok(initialLayout.sidebar.width >= 300 && initialLayout.sidebar.width <= 320);
    assert.ok(initialLayout.viewer.width > 1600);
    assert.ok(initialLayout.filmstrip.y > 850, 'filmstrip must sit below the main workspace');
    assert.ok(initialLayout.image.height <= initialLayout.viewer.height, 'photo must fit within the stage');

    assert.equal(await page.locator('#previous, #next, #fit, #mcp-open').count(), 0, 'duplicate controls and standalone AI entry are removed');
    assert.equal(await page.locator('.import-actions button').count(), 1, 'export uses the photo context menu and leaves only import in the sidebar');
    for (const [panel, content] of [['navigator-panel', 'navigator-preview'], ['folder-panel', 'folder-content']]) {
      await page.locator(`#${panel} h2`).click();
      assert.equal(await page.locator(`#${content}`).isVisible(), false);
      await page.locator(`#${panel} > summary`).focus();
      await page.keyboard.press('Enter');
      assert.equal(await page.locator(`#${content}`).isVisible(), true);
    }
    await page.click('#navigator-100');
    assert.equal(await page.locator('#navigator-preview').isVisible(), true, 'zoom buttons do not collapse the navigator');
    await page.click('#navigator-fit');
    const resizedPanels = [];
    for (const [handle, target, dimension, delta] of [
      ['sidebar-resize', 'sidebar', 'width', 35],
      ['navigator-resize', 'navigator-preview', 'height', -45],
      ['folder-resize', 'folder-content', 'height', -40],
    ]) {
      const original = (await page.locator(`#${target}`).boundingBox())[dimension];
      const grip = await page.locator(`#${handle}`).boundingBox();
      const x = grip.x + grip.width / 2, y = grip.y + grip.height / 2;
      await page.mouse.move(x, y); await page.mouse.down();
      await page.mouse.move(x + (dimension === 'width' ? delta : 0), y + (dimension === 'height' ? delta : 0));
      await page.mouse.up();
      assert.ok(Math.abs((await page.locator(`#${target}`).boundingBox())[dimension] - original - delta) < 1, `${handle} drag changes size`);
      await page.keyboard.press(dimension === 'width' ? 'ArrowRight' : 'ArrowDown');
      const size = (await page.locator(`#${target}`).boundingBox())[dimension];
      assert.ok(Math.abs(size - original - delta - 10) < 1, `${handle} supports keyboard resizing`);
      resizedPanels.push({ target, dimension, size });
    }
    await page.locator('#navigator-panel h2').click();
    await page.locator('#folder-panel h2').click();
    await page.waitForFunction(() => localStorage.getItem('openroom.sidebar.folder-panel') === 'false');
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => !document.querySelector('#app-shell').inert && document.querySelector('#full-image').src.includes('preview-1-1600'));
    for (const id of ['navigator-panel', 'folder-panel']) {
      assert.equal(await page.locator(`#${id}`).evaluate(panel => panel.open), false, 'collapse persists after reload');
      await page.locator(`#${id} h2`).click();
    }
    for (const { target, dimension, size } of resizedPanels) {
      assert.ok(Math.abs((await page.locator(`#${target}`).boundingBox())[dimension] - size) < 1, 'panel sizes persist after reload');
    }
    if (process.env.AE_UI_SCREENSHOT) await page.screenshot({ path: process.env.AE_UI_SCREENSHOT.replace('.png', '-resized.png') });
    await page.evaluate(() => {
      for (const key of Object.keys(localStorage)) if (key.startsWith('openroom.sidebar.')) localStorage.removeItem(key);
      for (const id of ['sidebar', 'navigator-preview', 'folder-content']) document.getElementById(id).removeAttribute('style');
    });
    await page.locator('#viewer').focus();

    await require('./test-review-ui.cjs')(page);
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => !document.querySelector('#app-shell').inert && document.querySelector('#full-image').src.includes('preview-1-1600'));

    // Native Lightroom navigation: G/E, linked navigator and filmstrip selection.
    await page.keyboard.press('g');
    await page.waitForSelector('#viewport:not([hidden])');
    await page.waitForFunction(() => document.querySelectorAll('.tile .thumb img').length >= 8);
    assert.ok(await page.locator('.tile').count() < 150, 'grid must virtualize');
    assert.equal(await page.locator('#extension, #formats, .filter-options').count(), 0);
    assert.equal(await page.locator('#pdf-controls, #page-prev, #page-next').count(), 0);
    await page.click('#filter-tabs [data-mode="metadata"]');
    const formatColumn = page.locator('.metadata-column').first();
    await formatColumn.locator('select').selectOption('extension');
    for (const extension of ['jpg', 'jpeg', 'psd', 'psb', 'tif', 'tiff']) {
      await formatColumn.locator('.facet-row').filter({hasText:new RegExp(`^${extension}\\d+$`)}).click();
      await page.waitForFunction(ext => document.querySelectorAll('.tile').length > 0 && [...document.querySelectorAll('.tile .badge')].every(badge => badge.textContent === ext.toUpperCase()), extension);
    }
    await page.click('#filter-tabs [data-mode="none"]');
    await page.waitForFunction(() => document.querySelector('.tile .badge')?.textContent === 'JPG');

    await page.locator('[data-folder="20261019撮影"] .folder-label').click();
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '1,200枚の写真');
    await page.locator('[data-folder="20261019撮影/選定"] .folder-label').click();
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '400枚の写真');
    await page.fill('#folder-search', '20261020');
    assert.equal(await page.locator('[data-folder="20261019撮影"]').count(), 0);
    assert.equal(await page.locator('[data-folder="20261020撮影"]').count(), 1);
    await page.fill('#folder-search', '');
    await page.locator('.catalog-panel').evaluate(node=>node.open=true);
    await page.click('#all');
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '2,000枚の写真');
    assert.equal(await page.locator('.tile[title="photo-0004.tif"] .adjustment-badge').count(), 1, 'grid shows the same saved adjustment status');
    await checkAdjustmentBadge('.tile[title="photo-0004.tif"]');
    if (process.env.AE_UI_SCREENSHOT) {
      await page.waitForFunction(() => [...document.querySelectorAll('.tile')].every(tile => {
        const image = tile.querySelector('img');
        return image?.complete && image.naturalWidth > 0;
      }));
      await page.screenshot({ path: process.env.AE_UI_SCREENSHOT.replace('.png', '-grid.png') });
      await page.locator('.tile[title="photo-0004.tif"]').screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png', '-badge-grid.png')});
    }
    await page.click('#toggle-inspector');
    await page.waitForFunction(() => !document.querySelector('#annotation-save').disabled);
    await page.click('#filter-tabs [data-mode="metadata"]');
    await page.fill('#annotation-memo', '未保存');
    await page.evaluate(() => window.listeners['library-preview-updated']({payload:1}));
    assert.equal(await page.inputValue('#annotation-memo'),'未保存','background preview updates must preserve unsaved annotations');
    decideDiscard(false);
    await formatColumn.locator('.facet-row').filter({hasText:/^psd\d+$/}).click();
    await discardResponse;
    await page.waitForFunction(() => document.querySelector('.metadata-column .facet-row.selected')?.textContent.startsWith('すべて'));
    assert.equal(await page.inputValue('#annotation-memo'), '未保存');
    decideDiscard(false);
    await page.click('#film-next');
    await discardResponse;
    assert.match(await page.textContent('#view-title'), /0000/);
    await page.fill('#annotation-tags', '旅行, <script>test</script>');
    await page.selectOption('#annotation-rating', '5');
    await page.fill('#annotation-memo', '保存と再読込の確認');
    await page.evaluate(() => { window.failSave = true; });
    await page.click('#annotation-save');
    await page.waitForFunction(() => document.querySelector('#annotation-status').textContent.includes('保存できませんでした'));
    assert.equal(await page.inputValue('#annotation-memo'), '保存と再読込の確認');
    await page.evaluate(() => { window.failSave = false; });
    await page.click('#annotation-save');
    await page.waitForFunction(() => document.querySelector('#annotation-status').textContent === '保存しました');
    await page.click('#filter-tabs [data-mode="metadata"]');
    await formatColumn.locator('select').selectOption('date');
    await page.click('#filter-tabs [data-mode="text"]');
    await page.fill('#search', '旅行');
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '1枚の写真');
    await page.fill('#search', '');
    await page.waitForFunction(() => document.querySelectorAll('.tile').length >= 8);
    await page.waitForFunction(() => !document.querySelector('#annotation-save').disabled);
    assert.equal(await page.inputValue('#annotation-rating'), '5');
    assert.equal(await page.inputValue('#annotation-memo'), '保存と再読込の確認');
    await page.keyboard.press('Tab');
    await page.click('#loupe-view');
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('preview-1-1600') && document.querySelector('#view-message').hidden);
    // Pause the large preview to inspect the first, low-resolution frame.
    async function fittedBounds() {
      return page.evaluate(() => {
        const image = document.querySelector('#full-image');
        const area = document.querySelector('#image-area');
        const style = getComputedStyle(area);
        const width = area.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
        const height = area.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
        const ratio = image.naturalWidth / image.naturalHeight;
        const rect = image.getBoundingClientRect();
        return {width:rect.width, height:rect.height, expectedWidth:Math.min(width,height*ratio), expectedHeight:Math.min(height,width/ratio)};
      });
    }
    const previewFitChecks = [];
    for (const id of [2, 3]) {
      await page.evaluate(() => { window.holdLargePreview = true; window.releaseLargePreview = null; });
      await page.click('#film-next');
      await page.waitForFunction(id => {
        const image = document.querySelector('#full-image');
        return window.releaseLargePreview && image.src.includes(`preview-${id}-384`) && image.complete && image.naturalWidth > 0 && !image.hidden;
      }, id);
      const thumbnailBounds = await fittedBounds();
      assert.ok(Math.abs(thumbnailBounds.width-thumbnailBounds.expectedWidth)<1 && Math.abs(thumbnailBounds.height-thumbnailBounds.expectedHeight)<1,
        `thumbnail ${id} must fit before large preview loads: ${JSON.stringify(thumbnailBounds)}`);
      await page.evaluate(() => { window.holdLargePreview = false; window.releaseLargePreview(); });
      await page.waitForFunction(id => {
        const image = document.querySelector('#full-image');
        return image.src.includes(`preview-${id}-1600`) && image.complete && document.querySelector('#view-message').hidden;
      }, id);
      const largeBounds = await fittedBounds();
      assert.ok(Math.abs(largeBounds.width-thumbnailBounds.width)<1 && Math.abs(largeBounds.height-thumbnailBounds.height)<1,
        'preview quality upgrade must preserve displayed size');
      previewFitChecks.push({id, thumbnail:thumbnailBounds, large:largeBounds});
    }
    await page.evaluate(() => { window.inlineLibraryPreview = true; });
    await page.click('#film-next');
    await page.waitForFunction(() => {
      const image = document.querySelector('#full-image');
      return image.src.startsWith('data:image/png;base64,') && image.complete && image.naturalWidth > 0 && document.querySelector('#view-message').hidden;
    });
    await page.evaluate(() => { window.inlineLibraryPreview = false; });
    await page.click('#film-previous');
    await page.click('#film-next');
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('preview-4-1600'));
    await page.click('#film-previous');
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('preview-3-1600'));
    await page.click('#film-previous');
    await page.click('#film-previous');
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('preview-1-1600') && document.querySelector('#view-message').hidden);
    const centeredPhotoPoint = () => page.evaluate(() => {
      const image = document.querySelector('#full-image').getBoundingClientRect();
      const area = document.querySelector('#image-area');
      const viewport = area.getBoundingClientRect();
      return {
        x: (viewport.left+area.clientLeft+area.clientWidth/2-image.left)/image.width,
        y: (viewport.top+area.clientTop+area.clientHeight/2-image.top)/image.height,
      };
    });
    const previewCallsBeforeZoom = await page.evaluate(() => calls.filter(call => call.command === 'preview').length);
    const fittedPhoto = await page.locator('#full-image').boundingBox();
    await page.mouse.click(fittedPhoto.x+fittedPhoto.width*.65, fittedPhoto.y+fittedPhoto.height*.35);
    assert.equal(await page.textContent('#zoom-level'), '100%', 'one photo click must zoom to 100%');
    const clickFocus = await centeredPhotoPoint();
    assert.ok(Math.abs(clickFocus.x-.65)<.002 && Math.abs(clickFocus.y-.35)<.002,
      `library zoom must center the clicked point in both axes: ${JSON.stringify(clickFocus)}`);
    await page.waitForTimeout(80);
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'preview').length), previewCallsBeforeZoom,
      'library zoom must keep the stored preview without requesting an original/high-resolution render');
    assert.ok(await page.evaluate(() => !calls.some(call => call.command === 'preview' && call.args.bound === 4096)));
    console.log('Library zoom keeps the stored preview and centers the clicked point.');
    await page.click('#full-image');
    assert.equal(await page.textContent('#zoom-level'), '', 'the next photo click must restore fit');
    const restoredFit = await fittedBounds();
    assert.ok(Math.abs(restoredFit.width-restoredFit.expectedWidth)<1 && Math.abs(restoredFit.height-restoredFit.expectedHeight)<1,
      'returning from 100% must fit the photo to the stage');
    const pinchChecks = await page.evaluate(() => {
      const image = document.querySelector('#full-image');
      const viewer = document.querySelector('#viewer');
      const area = document.querySelector('#image-area');
      const rect = image.getBoundingClientRect();
      const clientX = rect.left + rect.width / 2, clientY = rect.top + rect.height / 2;
      const state = () => {
        const box = image.getBoundingClientRect();
        return { width: box.width, height: box.height, x: (clientX-box.left)/box.width, y: (clientY-box.top)/box.height };
      };
      const wheel = (deltaY, ctrlKey = true, deltaMode = 0, target = viewer) => {
        const event = new WheelEvent('wheel', {deltaY, ctrlKey, deltaMode, clientX, clientY, bubbles:true, cancelable:true});
        target.dispatchEvent(event);
        return event.defaultPrevented;
      };
      const gesture = (type, scale, target = viewer) => {
        const event = new Event(type, {bubbles:true, cancelable:true});
        Object.assign(event, {scale, clientX, clientY});
        target.dispatchEvent(event);
        return event.defaultPrevented;
      };
      const start = state();
      const scrollCanceled = wheel(-30, false);
      const afterScroll = state();
      const pinchCanceled = wheel(-Math.log(1.5)*100);
      const enlarged = state();
      wheel(Math.log(1.5)*100);
      const shrunk = state();
      gesture('gesturestart', 1);
      gesture('gesturechange', 1.5);
      const webkitFirst = state();
      gesture('gesturechange', 2);
      const webkitSecond = state();
      wheel(-30);
      const afterDuplicate = state();
      gesture('gesturechange', 0);
      gesture('gesturechange', NaN);
      const afterInvalid = state();
      gesture('gesturechange', 1);
      gesture('gestureend', 1, document);
      wheel(-Math.log(1.5)*100/16, true, 1);
      const afterEnd = state();
      wheel(-Math.log(1.5)*100/area.clientHeight, true, 2);
      const pageUnits = state();
      wheel(-1000);
      const maxScale = image.getBoundingClientRect().width / Number(image.getAttribute('width'));
      wheel(1000);
      const minScale = image.getBoundingClientRect().width / Number(image.getAttribute('width'));
      document.querySelector('#navigator-100').click();
      area.scrollLeft = 400;
      area.scrollTop = 650;
      const offCenter = state();
      wheel(-Math.log(1.3)*100);
      const offCenterZoomed = state();
      document.querySelector('#navigator-fit').click();
      return {start, scrollCanceled, afterScroll, pinchCanceled, enlarged, shrunk, webkitFirst, webkitSecond,
        afterDuplicate, afterInvalid, afterEnd, pageUnits, maxScale, minScale, offCenter, offCenterZoomed};
    });
    const closeTo = (actual, expected, message, tolerance = .01) => assert.ok(Math.abs(actual-expected)<tolerance, `${message}: ${actual} vs ${expected}`);
    assert.equal(pinchChecks.scrollCanceled, true, 'photo panning must handle both wheel axes');
    // Layout rounds sizes to subpixels; allow at most 1/16 CSS pixel across the round trip.
    closeTo(pinchChecks.afterScroll.width, pinchChecks.start.width, 'ordinary scrolling must not zoom', 1/16);
    assert.equal(pinchChecks.pinchCanceled, true, 'pinching must prevent whole-page zoom');
    closeTo(pinchChecks.enlarged.width, pinchChecks.start.width*1.5, 'pinch out must enlarge from fit', 1/16);
    closeTo(pinchChecks.enlarged.x, pinchChecks.start.x, 'pinch must preserve the horizontal focus');
    closeTo(pinchChecks.enlarged.y, pinchChecks.start.y, 'pinch must preserve the vertical focus');
    closeTo(pinchChecks.shrunk.width, pinchChecks.start.width, 'pinch in must shrink smoothly', 1/16);
    closeTo(pinchChecks.webkitFirst.width, pinchChecks.start.width*1.5, 'WebKit pinch must enlarge', 1/16);
    closeTo(pinchChecks.webkitSecond.width, pinchChecks.start.width*2, 'WebKit scale must stay relative to gesture start', 1/16);
    closeTo(pinchChecks.afterDuplicate.width, pinchChecks.webkitSecond.width, 'wheel and gesture events must not double zoom', 1/16);
    closeTo(pinchChecks.afterInvalid.width, pinchChecks.webkitSecond.width, 'invalid gesture scales must be ignored', 1/16);
    closeTo(pinchChecks.afterEnd.width, pinchChecks.start.width*1.5, 'wheel pinching must resume after gesture end', 1/16);
    closeTo(pinchChecks.pageUnits.width, pinchChecks.start.width*2.25, 'wheel delta units must be normalized', 1/16);
    closeTo(pinchChecks.maxScale, 4, 'pinch must respect the 400% limit');
    closeTo(pinchChecks.minScale, .05, 'pinch must respect the 5% limit');
    closeTo(pinchChecks.offCenterZoomed.x, pinchChecks.offCenter.x, 'zoom must preserve an off-center horizontal focus');
    closeTo(pinchChecks.offCenterZoomed.y, pinchChecks.offCenter.y, 'zoom must preserve an off-center vertical focus');
    assert.equal(await page.textContent('#zoom-level'), '', 'fit button must still reset pinch zoom');
    console.log('Photo pinch checks passed: wheel, WebKit gestures, focus, limits, scrolling, fit reset.');
    async function checkPhotoPan(selector) {
      const checks = await page.locator(selector).evaluate(target => {
        const area = document.querySelector('#image-area');
        const image = document.querySelector('#full-image');
        document.querySelector('#navigator-100').click();
        for (let step = 0; step < 4; step++) document.querySelector('#zoom-in').click();
        const cases = [
          {name:'diagonal', deltaX:40, deltaY:60, x:40, y:60},
          {name:'reverse diagonal', deltaX:-40, deltaY:-60, x:-40, y:-60},
          {name:'horizontal', deltaX:40, deltaY:0, x:40, y:0},
          {name:'vertical', deltaX:0, deltaY:60, x:0, y:60},
          {name:'line units', deltaX:2, deltaY:-3, deltaMode:1, x:32, y:-48},
          {name:'page units', deltaX:.01, deltaY:.02, deltaMode:2, x:area.clientWidth*.01, y:area.clientHeight*.02},
        ];
        const results = cases.map(test => {
          area.scrollLeft = 200;
          area.scrollTop = 300;
          const before = image.getBoundingClientRect();
          const event = new WheelEvent('wheel', {...test, bubbles:true, cancelable:true});
          target.dispatchEvent(event);
          const after = image.getBoundingClientRect();
          return {name:test.name, x:before.left-after.left, y:before.top-after.top,
            expectedX:test.x, expectedY:test.y, width:after.width, originalWidth:before.width, canceled:event.defaultPrevented};
        });
        document.querySelector('#navigator-fit').click();
        return results;
      });
      for (const check of checks) {
        assert.equal(check.canceled, true, `${selector}: ${check.name} must prevent native axis locking`);
        closeTo(check.x, check.expectedX, `${selector}: ${check.name} horizontal movement`, 1);
        closeTo(check.y, check.expectedY, `${selector}: ${check.name} vertical movement`, 1);
        closeTo(check.width, check.originalWidth, `${selector}: panning must not change zoom`, 1/16);
      }
    }
    await checkPhotoPan('#full-image');
    console.log('Library photo panning checks passed.');
    async function checkNavigatorViewport(label) {
      const collapsed = !await page.locator('#sidebar').isVisible();
      if (collapsed) await page.click('#toggle-sidebar');
      await page.waitForFunction(() => {
        const thumbnail = document.querySelector('#navigator-image');
        return !thumbnail.hidden && thumbnail.complete && thumbnail.naturalWidth > 0;
      });
      assert.equal(await page.locator('#navigator-viewport').isVisible(), false, 'fit must hide the navigator frame');
      await page.click('#navigator-100');
      for (let step = 0; step < 3; step++) await page.click('#zoom-in');
      await page.waitForFunction(() => !document.querySelector('#navigator-viewport').hidden);
      const settle = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      const bounds = () => page.evaluate(() => {
        const frame = document.querySelector('#navigator-viewport').getBoundingClientRect();
        const thumbnail = document.querySelector('#navigator-image').getBoundingClientRect();
        const image = document.querySelector('#full-image').getBoundingClientRect();
        const area = document.querySelector('#image-area');
        return {left:frame.left, top:frame.top, right:frame.right, bottom:frame.bottom, width:frame.width, height:frame.height,
          thumbnailLeft:thumbnail.left+2, thumbnailTop:thumbnail.top+2, thumbnailRight:thumbnail.right-2, thumbnailBottom:thumbnail.bottom-2,
          expectedWidth:(thumbnail.width-4)*area.clientWidth/image.width, expectedHeight:(thumbnail.height-4)*area.clientHeight/image.height};
      });
      async function panTo(fraction) {
        await page.evaluate(fraction => {
          const area = document.querySelector('#image-area');
          area.scrollLeft = (area.scrollWidth-area.clientWidth)*fraction;
          area.scrollTop = (area.scrollHeight-area.clientHeight)*fraction;
        }, fraction);
        await settle();
        return bounds();
      }
      const center = await panTo(.5);
      closeTo((center.left+center.right)/2, (center.thumbnailLeft+center.thumbnailRight)/2, `${label}: centered horizontal viewport`, 1);
      closeTo((center.top+center.bottom)/2, (center.thumbnailTop+center.thumbnailBottom)/2, `${label}: centered vertical viewport`, 1);
      closeTo(center.width, center.expectedWidth, `${label}: viewport width must match the visible photo fraction`, 1);
      closeTo(center.height, center.expectedHeight, `${label}: viewport height must match the visible photo fraction`, 1);
      if (process.env.AE_UI_SCREENSHOT) await page.locator('.navigator').screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png', `-navigator-${label}.png`)});
      const start = await panTo(0), end = await panTo(1);
      closeTo(start.left, start.thumbnailLeft, `${label}: viewport must reach the left photo edge`, 1);
      closeTo(start.top, start.thumbnailTop, `${label}: viewport must reach the top photo edge`, 1);
      closeTo(end.right, end.thumbnailRight, `${label}: viewport must reach the right photo edge`, 1);
      closeTo(end.bottom, end.thumbnailBottom, `${label}: viewport must reach the bottom photo edge`, 1);
      await page.click('#zoom-in');
      await settle();
      assert.ok((await bounds()).width < center.width, `${label}: zooming in must shrink the navigator frame`);
      const size = page.viewportSize();
      await page.setViewportSize({width:size.width-100, height:size.height});
      const resized = await panTo(.5);
      closeTo(resized.width, resized.expectedWidth, `${label}: resizing must update the viewport frame`, 1);
      await page.setViewportSize(size);
      await page.locator('#navigator-fit').evaluate(button => button.click());
      await page.waitForFunction(() => document.querySelector('#navigator-viewport').hidden);
      if (collapsed) await page.click('#toggle-sidebar');
    }
    async function checkNavigationFocus(module) {
      const ready = () => page.waitForFunction(() => {
        const image = document.querySelector('#full-image');
        return !image.hidden && image.complete && image.naturalWidth > 0
          && image.alt === document.querySelector('#view-title').textContent
          && document.querySelector('#view-message').hidden;
      });
      const focus = () => page.evaluate(() => {
        const image = document.querySelector('#full-image'), area = document.querySelector('#image-area');
        const rect = image.getBoundingClientRect(), viewport = area.getBoundingClientRect();
        return { x:(viewport.left+area.clientLeft+area.clientWidth/2-rect.left)/rect.width,
          y:(viewport.top+area.clientTop+area.clientHeight/2-rect.top)/rect.height,
          scale:rect.width/Number(image.getAttribute('width')) };
      });
      await ready();
      await page.locator('#navigator-100').evaluate(button => button.click());
      await ready();
      for (const point of [{x:.5,y:.5}, {x:.7,y:.3}]) {
        await page.evaluate(point => {
          const area = document.querySelector('#image-area');
          document.querySelector('#viewer').dispatchEvent(new WheelEvent('wheel', {deltaY:0,bubbles:true,cancelable:true}));
          const rect = document.querySelector('#full-image').getBoundingClientRect(), viewport = area.getBoundingClientRect();
          area.scrollLeft += rect.left + point.x*rect.width - viewport.left - area.clientLeft - area.clientWidth/2;
          area.scrollTop += rect.top + point.y*rect.height - viewport.top - area.clientTop - area.clientHeight/2;
          document.activeElement.blur();
        }, point);
        const before = await focus();
        for (const [key, count] of [['ArrowRight', 1], ['ArrowLeft', 1], ['ArrowRight', 2], ['ArrowLeft', 2]]) {
          for (let step = 0; step < count; step++) {
            const title = await page.textContent('#view-title');
            await page.keyboard.press(key);
            await page.waitForFunction(title => document.querySelector('#view-title').textContent !== title, title);
          }
          await ready();
          const after = await focus();
          closeTo(after.x, before.x, `${module}: ${key} must preserve horizontal photo focus`, .003);
          closeTo(after.y, before.y, `${module}: ${key} must preserve vertical photo focus`, .003);
          closeTo(after.scale, before.scale, `${module}: ${key} must preserve zoom`, .001);
        }
      }
      await page.locator('#navigator-fit').evaluate(button => button.click());
      console.log(`${module} arrow navigation preserves center and panned focus, including rapid navigation.`);
    }
    await checkNavigationFocus('library');
    await checkNavigatorViewport('portrait');
    await page.click('#film-next');
    await page.waitForFunction(() => document.querySelector('#photo-details').textContent.includes('Camera 2'));
    await checkNavigatorViewport('landscape');
    await page.evaluate(() => { window.slow = true; });
    await page.click('#film-next');
    await page.click('#film-next');
    await page.waitForFunction(() => document.querySelector('#photo-details').textContent.includes('Camera 4') && document.querySelector('#view-message').hidden);
    assert.match(await page.textContent('#view-title'), /0003/);
    assert.ok((await page.getAttribute('#full-image', 'src')).includes('preview-4-1600'));
    assert.ok((await page.getAttribute('#navigator-image', 'src')).includes('preview-4-384'));
    await page.evaluate(() => { window.slow = false; });
    await page.click('#rating-stars [data-rating="4"]');
    await page.waitForFunction(() => document.querySelector('#annotation-status').textContent === '保存しました');
    assert.equal(await page.locator('.film-tile.active-photo .tile-rating').textContent(), '★★★★');
    await page.selectOption('#rating-filter', '4');
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '1枚の写真');
    assert.match(await page.textContent('#view-title'), /0003/);
    await page.selectOption('#rating-filter', '');
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '2,000枚の写真');

    // A catalog update must restart paging without losing the selected photo or draft.
    const pagingTitle = await page.textContent('#view-title');
    const pagingImage = await page.getAttribute('#full-image', 'src');
    await page.fill('#annotation-memo', 'Unsaved pagination draft');
    const pagingStart = await page.evaluate(() => {
      window.catalogGeneration++;
      const start = calls.length;
      document.querySelector('#filmstrip').scrollLeft = 200 * 83;
      return start;
    });
    await page.waitForFunction(() => document.querySelectorAll('.film-tile').length && [...document.querySelectorAll('.film-number')].some(number => Number(number.textContent) > 200));
    assert.equal(await page.textContent('#view-title'), pagingTitle);
    assert.equal(await page.getAttribute('#full-image', 'src'), pagingImage);
    assert.equal(await page.inputValue('#annotation-memo'), 'Unsaved pagination draft');
    assert.equal(await page.locator('#error').isVisible(), false);
    assert.equal(await page.locator('#discard-dialog').isVisible(), false);
    assert.deepEqual(await page.evaluate(start => calls.slice(start).filter(c => c.command === 'search').map(c => c.args.cursor?.offset || 0), pagingStart), [200, 0, 200]);
    assert.equal(await page.locator('.film-tile').evaluateAll(tiles => new Set(tiles.map(tile => tile.getAttribute('aria-label'))).size === tiles.length), true);
    await page.locator('#annotation-form').evaluate(form => form.requestSubmit());
    await page.waitForFunction(() => document.querySelector('#annotation-status').textContent === '保存しました');
    await page.getByRole('button', { name: /^フィルム photo-0200.jpg/ }).click();
    await page.waitForFunction(() => document.querySelector('#view-title').textContent.includes('0200'));
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('preview-201-1600') && document.querySelector('#view-message').hidden);
    await page.evaluate(() => { window.failImageOnce = 301; document.querySelector('#filmstrip').scrollLeft = 300 * 83; });
    await page.waitForFunction(() => !window.failImageOnce
      && [...document.querySelectorAll('.film-tile img')].some(image => image.src.includes(`${window.failedPreview}-jpeg`)
        && image.complete && image.naturalWidth > 0)
      && [...document.querySelectorAll('.film-tile')].every(tile => {
        const image = tile.querySelector('img');
        return image?.complete && image.naturalWidth > 0;
      }));
    assert.ok(await page.evaluate(() => {
      const id = Number(window.failedPreview.split('-')[1]);
      return calls.filter(call => call.command === 'preview' && call.args.id === id && !call.args.large).length >= 2;
    }), 'failed thumbnail must be requested again');
    assert.ok(await page.evaluate(() => [...document.querySelectorAll('.film-tile img')].some(image => image.src.includes(`${window.failedPreview}-jpeg`))),
      'browser-rejected preview must use the repaired JPEG path');
    await page.evaluate(() => { window.failThumbnailOnce = 351; document.querySelector('#filmstrip').scrollLeft = 350 * 83; });
    await page.waitForFunction(() => !window.failThumbnailOnce && [...document.querySelectorAll('.film-tile')].every(tile => {
      const image = tile.querySelector('img');
      return image?.complete && image.naturalWidth > 0;
    }));
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'preview' && !call.args.large && call.args.id === window.failedThumbnailId).length), 2,
      'temporary thumbnail command failure must retry once');
    await page.click('#grid-view');
    await page.waitForFunction(() => document.querySelector('.tile.selected')?.textContent.includes('0200'));
    await page.click('#all');
    await page.waitForFunction(() => document.querySelectorAll('.tile .thumb img').length === document.querySelectorAll('.tile').length);
    const cacheMilliseconds = await page.evaluate(async () => {
      const viewport = document.querySelector('#viewport');
      window.slow = true;
      viewport.scrollTop = 5000;
      await new Promise(resolve => setTimeout(resolve, 40));
      const start = performance.now();
      viewport.scrollTop = 0;
      while (performance.now() - start < 500) {
        await new Promise(resolve => requestAnimationFrame(resolve));
        const tiles = [...document.querySelectorAll('.tile')];
        if (tiles.length && tiles.every(tile => tile.querySelector('img')) && tiles[0].textContent.includes('0000')) return performance.now() - start;
      }
      return 500;
    });
    assert.ok(cacheMilliseconds < 150, `cached tiles blocked by cold jobs: ${cacheMilliseconds}ms`);
    await page.evaluate(() => { window.slow = false; });
    await page.click('#loupe-view');
    await page.click('#toggle-inspector');
    await page.click('#toggle-sidebar');
    assert.equal(await page.locator('#sidebar').isVisible(), false);
    await page.click('#toggle-sidebar');
    await page.click('#toggle-modulebar');
    assert.equal(await page.locator('#module-bar').isVisible(), false);
    await page.click('#toggle-modulebar');
    await page.click('#toggle-filmstrip');
    assert.equal(await page.locator('#filmstrip').isVisible(), false);
    await page.click('#toggle-filmstrip');
    for (const size of [{ width: 1320, height: 880 }, { width: 900, height: 600 }]) {
      await page.setViewportSize(size);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'workspace must fit the supported window size');
      const stage = await page.locator('#viewer').boundingBox();
      assert.ok(stage.width > 500 && stage.height > 300, 'photo must retain usable space');
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      const fit = await fittedBounds();
      assert.ok(Math.abs(fit.width-fit.expectedWidth)<1 && Math.abs(fit.height-fit.expectedHeight)<1, 'window resize must refit the image');
      if (process.env.AE_UI_SCREENSHOT) await page.screenshot({ path: process.env.AE_UI_SCREENSHOT.replace('.png', `-${size.width}.png`) });
    }
    await page.setViewportSize({width:1320,height:880});
    assert.ok(await page.evaluate(() => calls.some(call => call.command === 'prepare_library_previews' && typeof call.args.avifSupported === 'boolean')),
      'opening a catalog starts preview preparation without waiting for photo selection');
    assert.ok(await page.evaluate(() => !calls.some(call => call.command === 'develop_preview')),
      'library selection and zoom must never start development');
    await page.evaluate(() => {window.holdOriginalLoad = true;});
    assert.deepEqual(await page.evaluate(() => calls.filter(c => c.command === 'set_develop_prefetch').at(-1).args.ids), [1], 'library preloads only the selected photo');
    await page.click('#high-quality');
    await page.waitForFunction(() => window.releaseOriginalLoad);
    assert.equal(await page.locator('#full-image').isVisible(),false,'the library preview must not be presented as the completed development image');
    assert.match(await page.textContent('#develop-status'),/原本/);
    await page.evaluate(() => {window.holdOriginalLoad = false;window.releaseOriginalLoad();});
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-'));
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-1-0-1600'));
    assert.deepEqual(await page.evaluate(() => calls.filter(c => c.command === 'set_develop_prefetch').at(-1).args.ids), [1, 2, 3, 4], 'at the beginning, prefetch only the three available successors');
    for (const [number, ids] of [[4, [4,5,3,6,2,7,1]], [8, [8,9,7,10,6,11,5]], [1, [1,2,3,4]]]) {
      await page.locator('.film-tile').filter({has:page.locator('.film-number', {hasText:new RegExp(`^${number}$`)})}).click();
      await page.waitForFunction(number => calls.filter(c => c.command === 'set_develop_prefetch').at(-1)?.args.ids[0] === number, number);
      assert.deepEqual(await page.evaluate(() => calls.filter(c => c.command === 'set_develop_prefetch').at(-1).args.ids), ids, 'navigation replaces the window with nearest neighbors first');
    }
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-1-0-1600'));
    assert.equal(await page.locator('#develop-panel').isVisible(), true);
    assert.equal(await page.locator('#sidebar').isVisible(), false, 'first Develop view gives the photo the left panel space');
    assert.equal(await page.locator('#photo-flags').isVisible(), false, 'Develop uses comparison tools instead of Library attributes');
    await page.click('#toggle-sidebar');
    await page.click('#toggle-inspector');
    assert.equal(await page.getAttribute('#toggle-inspector', 'aria-expanded'), 'false');
    await page.click('#toggle-inspector');
    assert.equal(await page.getAttribute('#toggle-inspector', 'aria-expanded'), 'true');
    assert.equal(await page.locator('#library-inspector').isVisible(), false);
    assert.deepEqual(await page.locator('#module-bar nav button').allTextContents(), ['ライブラリ', '現像']);
    await checkNavigationFocus('develop');
    const sliderTracks = await page.evaluate(() => Object.fromEntries([
      'raw_temperature', 'raw_tint', 'saturation', 'vibrance', 'exposure', 'texture',
      'mixer-0-0', 'mixer-0-1', 'mixer-0-2', 'grading-0-0', 'grading-0-1', 'grading-0-2',
    ].map(key => [key, getComputedStyle(document.getElementById(`develop-${key}`)).getPropertyValue('--range-track')])));
    for (const [key, track] of Object.entries(sliderTracks)) assert.match(track, /linear-gradient/, `${key} has no direction colors`);
    assert.notEqual(sliderTracks.raw_temperature, sliderTracks.raw_tint);
    for (const id of ['develop-saturation', 'local-saturation']) {
      for (const suffix of ['', '-value']) {
        assert.equal(await page.getAttribute(`#${id}${suffix}`, 'min'), '-100');
        assert.equal(await page.getAttribute(`#${id}${suffix}`, 'max'), '300');
      }
    }
    await page.fill('#develop-saturation-value', '300');
    await page.locator('#develop-saturation-value').blur();
    await page.waitForFunction(() => calls.some(c => c.command === 'develop_preview' && c.args.settings.saturation === 300));
    assert.equal(await page.inputValue('#develop-saturation'), '300');
    await page.click('#develop-undo');
    await page.waitForFunction(() => document.getElementById('develop-saturation-value').value === '0');
    await page.fill('#develop-saturation-value', '125');
    await page.locator('#develop-saturation-value').dblclick();
    assert.equal(await page.inputValue('#develop-saturation'), '0', 'double-click resets the adjustment');
    assert.equal(await page.inputValue('#develop-saturation-value'), '0', 'focused numeric input must show the reset value immediately');
    await page.locator('#develop-saturation-value').blur();
    assert.match(await page.locator('#develop-sharpen_radius').locator('..').getAttribute('title'), /シャープ量/);
    await page.evaluate(() => {
      const range = document.getElementById('develop-texture');
      range.dispatchEvent(new Event('pointerdown', {bubbles:true}));
      range.value = '100'; range.dispatchEvent(new Event('input', {bubbles:true}));
      range.dispatchEvent(new Event('change', {bubbles:true}));
    });
    await page.waitForFunction(() => calls.some(c => c.command === 'develop_preview' && c.args.settings.texture === 100));
    assert.equal(await page.inputValue('#develop-texture-value'), '100');
    await page.click('#develop-undo');
    await page.waitForFunction(() => document.getElementById('develop-texture-value').value === '0');
    async function exposure(value) {
      await page.locator('#develop-exposure-value').fill(String(value));
      await page.locator('#develop-exposure-value').dispatchEvent('input');
      await page.locator('#develop-exposure-value').blur();
    }
    // Keep the first recipe dirty while exercising preview and explicit retry behavior.
    await page.evaluate(() => { window.failDevelopSave = true; });
    await page.locator('#develop-exposure-value').click();
    await page.keyboard.press('ControlOrMeta+a');
    await page.keyboard.type('0.75');
    await page.locator('#develop-exposure-value').blur();
    assert.equal(await page.inputValue('#develop-exposure-value'), '0.75', 'typing a fraction must preserve its decimal point');
    await exposure(1);
    await page.waitForFunction(() => calls.some(call => call.command === 'develop_preview' && call.args.settings.exposure === 1));
    await exposure(2);
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-1-2-1600'));
    assert.equal(await page.locator('#develop-save').isEnabled(), true);
    assert.ok(await page.evaluate(() => calls.some(call => call.command === 'develop_preview' && call.args.settings.exposure === 2)));
    await page.click('#develop-undo');
    assert.equal(await page.inputValue('#develop-exposure-value'), '1');
    await page.click('#develop-redo');
    assert.equal(await page.inputValue('#develop-exposure-value'), '2');
    await page.click('#develop-before');
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-1-0-1600'));
    assert.equal(await page.inputValue('#develop-exposure-value'), '2');
    await page.click('#develop-before');
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-1-2-1600'));
    await page.evaluate(() => { window.failDevelopSave = true; });
    await page.click('#develop-save');
    await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存できません'));
    assert.equal(await page.locator('#develop-save').isEnabled(), true);
    await page.evaluate(() => {window.failDevelopPreview = true;});
    await page.click('#high-quality');
    await page.waitForFunction(() => document.querySelector('#error').textContent.includes('Injected preview failure'));
    assert.match(await page.textContent('#develop-status'), /保存できません/, 'preview failures must not hide save errors');
    await page.evaluate(() => {window.failDevelopPreview = false;});
    await page.evaluate(() => {
      window.failDevelopSave = false; window.holdThumbnail = true;
      window.otherThumbnailCount = calls.filter(call => call.command === 'preview' && !call.args.large && call.args.id !== 1).length;
    });
    await page.click('#develop-save');
    await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存済み'));
    await page.waitForFunction(() => document.querySelector('.film-tile.selected .adjustment-badge'));
    assert.equal(await page.locator('#develop-save').isDisabled(), true);
    assert.equal(await page.evaluate(()=>Boolean(window.releaseThumbnail)), false, 'saving alone must not refresh the filmstrip');
    await page.click('#film-next');
    await page.waitForFunction(() => Boolean(window.releaseThumbnail));
    await page.click('#film-previous');
    await page.waitForFunction(()=>document.querySelector('#develop-exposure-value').value==='2');
    await exposure(3);
    await page.click('#develop-save');
    await page.waitForFunction(()=>document.querySelector('#develop-save').disabled);
    await page.click('#film-next'); await page.click('#film-previous');
    await page.waitForFunction(() => document.querySelector('.film-tile.selected img')?.src.includes('384-r2'));
    await page.evaluate(() => { window.releaseThumbnail(); });
    // Force a later redraw: the obsolete reply must not poison the ready cache.
    await page.evaluate(() => { document.querySelector('#filmstrip').scrollLeft = 1500; });
    await page.waitForFunction(() => !document.querySelector('.film-tile.selected'));
    await page.evaluate(() => { document.querySelector('#filmstrip').scrollLeft = 0; });
    await page.waitForFunction(() => document.querySelector('.film-tile.selected img')?.src.includes('384-r2'));
    await exposure(2);
    const otherCount = await page.evaluate(() => calls.filter(call => call.command === 'preview' && !call.args.large && call.args.id !== 1).length);
    await page.click('#develop-save');
    await page.waitForFunction(() => document.querySelector('#develop-save').disabled);
    await page.click('#film-next'); await page.click('#film-previous');
    await page.waitForFunction(() => document.querySelector('.film-tile.selected img')?.src.includes('384-r3'));
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'preview' && !call.args.large && call.args.id !== 1).length), otherCount, 'saving one recipe must not reload unrelated thumbnails');
    await page.click('#module-library');
    assert.deepEqual(await page.evaluate(() => calls.filter(c => c.command === 'set_develop_prefetch').at(-1).args.ids), [1], 'returning to Library discards neighboring development inputs');
    await page.click('#module-develop');
    await page.waitForFunction(() => document.querySelector('#develop-exposure-value').value === '2');
    await page.waitForFunction(() => document.querySelector('#full-image').alt === document.querySelector('#view-title').textContent);
    await page.evaluate(() => { window.failDevelopSave = true; });
    await page.click('#develop-reset');
    assert.equal(await page.inputValue('#develop-exposure-value'), '0');
    await page.evaluate(() => window.listeners['close-requested']());
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'close_window').length), 0, 'save failure must stop close');
    assert.equal(await page.inputValue('#develop-exposure-value'), '0', 'save failure must preserve edits');
    await page.click('#film-next');
    await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存できません'));
    assert.match(await page.textContent('#view-title'), /0000/);
    await page.click('#module-library');
    await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存できません'));
    assert.equal(await page.locator('#develop-panel').isVisible(), true);
    await page.click('#develop-undo');
    await page.evaluate(() => { window.failDevelopSave = false; });
    await page.click('#high-quality');
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-1-2-0'));
    assert.equal(await page.locator('#develop-export, #batch-export').count(), 0);
    await page.locator('.film-tile').filter({has:page.locator('.film-number', {hasText:/^1$/})}).click({button:'right'});
    await page.click('#photo-export');
    await page.click('#export-start');
    assert.match(await page.textContent('#export-result'), /フォルダを選択/);
    await page.click('#export-browse');
    await page.click('#export-start');
    await page.waitForFunction(() => document.querySelector('#export-result').textContent.includes('1/1'));
    assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'export_develop_batch').at(-1).args.request.options.border), null);
    assert.equal(await page.locator('#export-border-ratio').isVisible(), false);
    await page.click('#export-close');
    assert.equal(await page.locator('.panel-unavailable').count(), 0, 'unimplemented panels must not remain');
    for (const [group, field, value] of [['geometry', 'perspective_v', '15'], ['effects', 'vignette', '-30']]) {
      await page.locator(`#develop-${group}`).evaluate(node => { node.closest('details').open = true; });
      const input = page.locator(`#develop-${field}-value`);
      assert.equal(await input.isVisible(), true);
      assert.equal(await input.isEnabled(), true);
      await input.fill(value); await input.blur();
      await page.waitForFunction(({field,value}) => calls.filter(c => c.command === 'develop_preview').at(-1)?.args.settings[field] === Number(value), {field,value});
      await page.click('#develop-undo');
      await page.locator(`#develop-${group}`).evaluate(node => { node.closest('details').open = false; });
    }
    await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存済み'));
    const previewsBeforeToolSwitch = await page.evaluate(() => calls.filter(call => call.command === 'develop_preview').length);
    await page.evaluate(() => { window.failDevelopPreview = true; });
    await page.locator('.develop-toolstrip [data-tool="crop"]').click();
    await page.click('#crop-straighten');
    await page.waitForTimeout(100);
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'develop_preview').length), previewsBeforeToolSwitch, 'opening straighten must not reread an unchanged photo on an offline drive');
    assert.equal(await page.locator('#develop-angle-value').isVisible(), true);
    assert.equal(await page.locator('#develop-rotate').isVisible(), false);
    await page.locator('#crop-advanced > summary').click();
    assert.equal(await page.locator('#develop-rotate').isVisible(), true);
    await page.locator('.develop-toolstrip [data-tool="crop"]').click();
    await page.waitForTimeout(100);
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'develop_preview').length), previewsBeforeToolSwitch, 'switching between uncropped tools must reuse the displayed preview');
    await page.evaluate(() => { window.failDevelopPreview = false; });
    assert.equal(await page.locator('summary').filter({hasText:/^切り抜き・回転$/}).count(), 0, 'crop and rotation must have only the upper tool controls');
    assert.equal(await page.locator('#develop-rotate').count(), 1);
    await page.click('#develop-rotate');
    await page.waitForFunction(() => calls.filter(call => call.command === 'develop_preview').at(-1)?.args.settings.quarter_turns === 1);
    await page.click('#develop-undo');
    await page.waitForFunction(() => calls.filter(call => call.command === 'develop_preview').at(-1)?.args.settings.quarter_turns === 0);
    await page.fill('#develop-angle-value', '5');
    await page.locator('#develop-angle-value').blur();
    await page.waitForFunction(() => calls.filter(call => call.command === 'develop_preview').at(-1)?.args.settings.angle === 5);
    await page.click('#develop-undo');
    await page.fill('#develop-crop-2-value', '50');
    await page.fill('#develop-crop-0-value', '30');
    await page.locator('#develop-crop-2').dblclick();
    await page.locator('#develop-crop-0-value').blur();
    assert.equal(await page.inputValue('#develop-crop-2-value'), '100');
    assert.equal(await page.inputValue('#develop-crop-0-value'), '0', 'resetting crop width must keep crop inside image');
    await page.waitForFunction(() => {
      const crop = calls.filter(call => call.command === 'develop_preview').at(-1)?.args.settings.crop;
      return crop && crop[0] + crop[2] <= 1 && crop[2] === 1;
    });
    const savesBefore = await page.evaluate(() => calls.filter(call => call.command === 'save_develop').length);
    await exposure(3);
    await exposure(4);
    await page.waitForFunction(() => document.querySelector('#develop-save').disabled && document.querySelector('#develop-status').textContent.includes('保存済み'));
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'save_develop').length), savesBefore + 1, 'rapid input must coalesce into one autosave');
    await page.evaluate(() => { window.holdDevelopSave = true; });
    await exposure(3);
    await page.waitForFunction(() => Boolean(window.releaseDevelopSave));
    assert.equal(await page.locator('#develop-exposure-value').isEnabled(), true, 'autosaving must not freeze editing');
    await exposure(1.5);
    const exportsBefore = await page.evaluate(() => calls.filter(call => call.command === 'export_develop_batch').length);
    await page.locator('.film-tile').filter({has:page.locator('.film-number', {hasText:/^1$/})}).click({button:'right'});
    await page.click('#photo-export');
    assert.equal(await page.locator('#export-dialog').isVisible(), false, 'export waits for pending recipe save');
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'export_develop_batch').length), exportsBefore);
    await page.evaluate(() => window.releaseDevelopSave());
    await page.waitForFunction(() => document.querySelector('#develop-save').disabled && document.querySelector('#develop-status').textContent.includes('保存済み'));
    assert.equal(await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop', {id:1})).settings.exposure), 1.5, 'edits during a write must survive');
    await page.locator('#export-dialog[open]').waitFor();
    await page.click('#export-close');
    await exposure(2.25);
    await page.click('#film-next');
    await page.waitForFunction(() => document.querySelector('#view-title').textContent.includes('0001'));
    assert.equal(await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop', {id:1})).settings.exposure), 2.25, 'navigation must flush before changing selection');
    await page.click('#film-previous');
    await page.waitForFunction(() => document.querySelector('#develop-exposure-value').value === '2.25');
    await exposure(1.75);
    await page.locator('#catalog-manage').evaluate(panel => panel.open = true);
    await page.click('#catalog-copy');
    await page.waitForFunction(() => document.querySelector('#status').textContent.includes('/backups/catalog-copy.openroom'));
    assert.equal(await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop', {id:1})).settings.exposure), 1.75, 'snapshot must flush pending edits');
    await page.click('#catalog-location');
    assert.equal(await page.locator('#catalog-location').getAttribute('title'), '/catalogs/catalog.db');
    await exposure(2.5);
    await page.evaluate(() => window.listeners['close-requested']());
    assert.equal(await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop', {id:1})).settings.exposure), 2.5, 'normal close must flush the latest edit');
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'close_window').length), 1);
    // Both gestures remain in history even when one autosave commits them together.
    await exposure(3.25); await exposure(3.75);
    await page.click('#develop-save');
    await page.waitForFunction(() => document.querySelector('#develop-status').textContent.includes('保存済み'));
    const history = await page.evaluate(() => __TAURI__.core.invoke('develop_history', {id:1,before:null}));
    const previous = history[1];
    assert.equal(await page.evaluate(async historyId => (await __TAURI__.core.invoke('history_settings', {id:1,historyId})).exposure, previous.id), 3.25);
    await page.locator('#develop-history-list button').nth(1).click();
    await page.waitForFunction(() => document.querySelector('#develop-exposure-value').value === '3.25' && document.querySelector('#develop-save').disabled);
    await page.click('#module-library'); await page.click('#module-develop');
    await page.waitForFunction(() => document.querySelector('#develop-history-list button').textContent.includes('履歴から復元'));
    assert.equal(await page.inputValue('#develop-exposure-value'), '3.25');
    const afterRestore = await page.evaluate(() => __TAURI__.core.invoke('develop_history', {id:1,before:null}));
    assert.equal(afterRestore.length, Math.min(11, history.length + 1), 'restoring retains at most ten previous states');
    await exposure(4.25);
    await page.click('#catalog-open'); // cancel the native picker
    await page.waitForFunction(() => !document.body.dataset.catalogBusy);
    assert.equal(await page.evaluate(async () => (await __TAURI__.core.invoke('get_develop', {id:1})).settings.exposure), 4.25);
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'restart_catalog').length), 0);
    await page.evaluate(() => {window.failCatalog = true;});
    await page.click('#catalog-open');
    await page.waitForFunction(() => !document.body.dataset.catalogBusy);
    assert.equal(await page.locator('#develop-exposure-value').isEnabled(), true);
    await page.evaluate(() => {window.failCatalog = false;});
    // Exercise the new controls through real pointer events, persistence, and selection.
    await page.setViewportSize({width:1320,height:880});
    await page.locator('.develop-toolstrip [data-tool="crop"]').click();
    await page.selectOption('#crop-ratio','0');
    await page.waitForFunction(()=>document.querySelector('#develop-overlay').getBoundingClientRect().width>100);
    const overlayPinch = await page.locator('#develop-overlay').evaluate(overlay => {
      const image = document.querySelector('#full-image');
      const before = image.getBoundingClientRect();
      const event = new WheelEvent('wheel', {ctrlKey:true, deltaY:-Math.log(1.25)*100,
        clientX:before.left+before.width/2, clientY:before.top+before.height/2, bubbles:true, cancelable:true});
      overlay.dispatchEvent(event);
      const ratio = image.getBoundingClientRect().width / before.width;
      document.querySelector('#navigator-fit').click();
      return {ratio, canceled:event.defaultPrevented};
    });
    closeTo(overlayPinch.ratio, 1.25, 'pinch must work over Develop editing overlays');
    assert.equal(overlayPinch.canceled, true);
    console.log('Develop overlay pinch check passed.');
    await checkPhotoPan('#develop-overlay');
    console.log('Develop overlay panning checks passed.');
    await checkNavigatorViewport('develop');
    assert.equal(await page.textContent('#photo-tool-heading'), '切り抜きと角度補正');
    const paperRatios = [
      ['A判・B判（1:√2）', Math.SQRT2],
      ['L判 89×127 mm', 127/89], ['2L判 127×178 mm', 178/127], ['KG 102×152 mm', 152/102],
      ['六つ切 203×254 mm', 254/203], ['六つ切ワイド 203×305 mm', 305/203],
      ['四つ切 254×305 mm', 305/254], ['四つ切ワイド 254×366 mm', 366/254],
    ];
    const ratioOptions = await page.locator('#crop-ratio option').evaluateAll(options => options.map(option => ({label:option.label, value:option.value})));
    assert.equal(ratioOptions.filter(option => /A判・B判/.test(option.label)).length, 1);
    assert.ok(!ratioOptions.some(option => /A3|A4|B4|B5/.test(option.label)));
    assert.equal(new Set(ratioOptions.map(option => option.value)).size, ratioOptions.length, 'same ratio must appear only once');
    async function savedCrop() {
      if (await page.locator('#develop-save').isEnabled()) await page.click('#develop-save');
      await page.waitForFunction(() => document.querySelector('#develop-save').disabled && document.querySelector('#develop-status').textContent.includes('保存済み'));
      return page.evaluate(async () => {
        const settings = (await __TAURI__.core.invoke('get_develop', {id:1})).settings;
        const image = document.querySelector('#full-image');
        return {crop:settings.crop, ratio:settings.crop[2]/settings.crop[3]*Number(image.getAttribute('width'))/Number(image.getAttribute('height')),
          quarterTurns:settings.quarter_turns};
      });
    }
    await page.click('#crop-landscape');
    for (const [label, ratio] of paperRatios) {
      await page.selectOption('#crop-ratio', {label});
      const value = await savedCrop();
      closeTo(value.ratio, ratio, `${label}: paper ratio must update the existing crop`, 1e-6);
      assert.ok(value.crop[0]>=0 && value.crop[1]>=0 && value.crop[0]+value.crop[2]<=1.00001 && value.crop[1]+value.crop[3]<=1.00001);
    }
    await page.selectOption('#crop-ratio', {label:'A判・B判（1:√2）'});
    const landscapeCrop = await savedCrop();
    if (process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-crop-landscape.png')});
    await page.click('#crop-portrait');
    const portraitCrop = await savedCrop();
    closeTo(portraitCrop.ratio, 1/Math.SQRT2, 'portrait button must invert the paper ratio', 1e-6);
    assert.equal(portraitCrop.quarterTurns, landscapeCrop.quarterTurns, 'changing crop orientation must not rotate the photo');
    assert.equal(await page.getAttribute('#crop-portrait', 'aria-pressed'), 'true');
    if (process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-crop-portrait.png')});
    await page.click('#develop-undo');
    closeTo((await savedCrop()).ratio, Math.SQRT2, 'undo must restore crop orientation', 1e-6);
    await page.click('#develop-redo');
    closeTo((await savedCrop()).ratio, 1/Math.SQRT2, 'redo must restore portrait crop', 1e-6);
    const ratioOverlay = await page.locator('#develop-overlay').boundingBox();
    await page.mouse.move(ratioOverlay.x+ratioOverlay.width*.05, ratioOverlay.y+ratioOverlay.height*.05);
    await page.mouse.down();
    await page.mouse.move(ratioOverlay.x+ratioOverlay.width*.25, ratioOverlay.y+ratioOverlay.height*.25, {steps:5});
    await page.mouse.up();
    const drawnCrop = await savedCrop();
    closeTo(drawnCrop.ratio, 1/Math.SQRT2, 'drawing a crop must keep the selected portrait ratio', 1e-6);
    await page.selectOption('#crop-ratio', '0');
    await page.click('#crop-landscape');
    const flippedCrop = await savedCrop();
    closeTo(flippedCrop.ratio, Math.SQRT2, 'free crop orientation must also swap width and height', 1e-6);
    closeTo(flippedCrop.crop[2]*flippedCrop.crop[3], drawnCrop.crop[2]*drawnCrop.crop[3], 'orientation must preserve crop area when it fits', 1e-6);
    for (const axis of [0, 1]) closeTo(flippedCrop.crop[axis]+flippedCrop.crop[axis+2]/2,
      drawnCrop.crop[axis]+drawnCrop.crop[axis+2]/2, 'orientation must preserve the crop center when it fits', 1e-6);
    async function cropHandleBounds(corner) {
      // Rendering can replace the SVG handle between separate visibility and bounds queries.
      return (await page.waitForFunction(corner => {
        const rect = document.querySelector(`[data-crop-corner="${corner}"]`)?.getBoundingClientRect();
        return rect?.width > 0 && rect.height > 0 ? {x:rect.x,y:rect.y,width:rect.width,height:rect.height} : null;
      }, corner)).jsonValue();
    }
    for (const [preset, orientation, expectedRatio] of [['0', null, null], ['1', null, 1], ['1.5', 'landscape', 1.5], ['1.5', 'portrait', 2/3]]) {
      for (const [corner, right, bottom] of [['nw',0,0], ['ne',1,0], ['sw',0,1], ['se',1,1]]) {
        await page.selectOption('#crop-ratio', '0');
        await page.evaluate(() => {
          for (const [index, value] of [[2,50],[3,50],[0,20],[1,20]]) {
            const input = document.querySelector(`#develop-crop-${index}-value`);
            input.value = value; input.dispatchEvent(new Event('input', {bubbles:true})); input.dispatchEvent(new FocusEvent('focusout', {bubbles:true}));
          }
        });
        await page.selectOption('#crop-ratio', preset);
        if (orientation) await page.click(`#crop-${orientation}`);
        const before = await savedCrop();
        const box = await page.locator('#develop-overlay').boundingBox();
        const handle = await cropHandleBounds(corner);
        await page.mouse.move(handle.x+handle.width/2, handle.y+handle.height/2);
        await page.mouse.down();
        await page.mouse.move(box.x+(before.crop[0]+right*before.crop[2]+(right ? -.06 : .06))*box.width,
          box.y+(before.crop[1]+bottom*before.crop[3]+(bottom ? -.1 : .1))*box.height, {steps:5});
        await page.mouse.up();
        const resized = await savedCrop();
        assert.ok(resized.crop[2]<before.crop[2] && resized.crop[3]<before.crop[3], `${preset}/${orientation}/${corner}: the corner must shrink the existing crop: ${JSON.stringify({before,resized,box,handle})}`);
        for (const [axis, side] of [[0,right],[1,bottom]]) closeTo(resized.crop[axis]+(1-side)*resized.crop[axis+2],
          before.crop[axis]+(1-side)*before.crop[axis+2], `${corner}: the opposite corner must stay fixed`, 1e-6);
        if (expectedRatio) closeTo(resized.ratio, expectedRatio, `${corner}: resizing must preserve the selected ratio`, 1e-6);
        // Expand to the image edge, then drag past the opposite corner to the minimum size.
        for (const destination of ['edge', 'cross']) {
          const currentHandle = await cropHandleBounds(corner);
          await page.mouse.move(currentHandle.x+currentHandle.width/2, currentHandle.y+currentHandle.height/2);
          await page.mouse.down();
          const endX = destination === 'edge' ? right : before.crop[0]+(1-right)*before.crop[2]+(right ? -.02 : .02);
          const endY = destination === 'edge' ? bottom : before.crop[1]+(1-bottom)*before.crop[3]+(bottom ? -.02 : .02);
          await page.mouse.move(box.x+endX*box.width, box.y+endY*box.height, {steps:5});
          await page.mouse.up();
          const bounded = await savedCrop();
          assert.ok(bounded.crop[0]>=-1e-9 && bounded.crop[1]>=-1e-9 && bounded.crop[0]+bounded.crop[2]<=1.000001 && bounded.crop[1]+bounded.crop[3]<=1.000001, `${corner}: resize must stay inside the photo`);
          assert.ok(bounded.crop[2]>=.01-1e-9 && bounded.crop[3]>=.01-1e-9, `${corner}: resize must respect minimum dimensions`);
          if (destination === 'cross') closeTo(Math.min(bounded.crop[2], bounded.crop[3]), .01, `${corner}: crossing the anchor must stop at the minimum size`, 1e-6);
          for (const [axis, side] of [[0,right],[1,bottom]]) closeTo(bounded.crop[axis]+(1-side)*bounded.crop[axis+2],
            before.crop[axis]+(1-side)*before.crop[axis+2], `${corner}: bounds must preserve the opposite corner`, 1e-6);
          if (expectedRatio) closeTo(bounded.ratio, expectedRatio, `${corner}: bounds must preserve the ratio`, 1e-6);
        }
      }
    }
    console.log('All four crop corners passed free, square, portrait, landscape, image-edge and minimum-size checks.');
    await page.selectOption('#crop-ratio', '0');
    await page.click('#crop-clear');
    await page.evaluate(() => {
      for (const point of [2, 3]) {
        const input = document.querySelector(`#develop-crop-${point}-value`);
        input.value = '1'; input.dispatchEvent(new Event('input', {bubbles:true})); input.dispatchEvent(new FocusEvent('focusout', {bubbles:true}));
      }
    });
    await page.selectOption('#crop-ratio', '1.25');
    const minimumCrop = await savedCrop();
    assert.ok(minimumCrop.crop[2]>=.01 && minimumCrop.crop[3]>=.01, 'paper ratios must respect the minimum crop size');
    closeTo(minimumCrop.ratio, 4/5, 'small portrait crop must keep the selected ratio', 1e-6);
    await page.selectOption('#crop-ratio', '0');
    await page.click('#crop-clear');
    await page.evaluate(() => {
      document.querySelector('#navigator-100').click();
      for (let step=0; step<4; step++) document.querySelector('#zoom-in').click();
      const area = document.querySelector('#image-area');
      area.scrollLeft = (area.scrollWidth-area.clientWidth)/2;
      area.scrollTop = (area.scrollHeight-area.clientHeight)/2;
    });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const overlayClipped = await page.evaluate(() => {
      const ids = ['module-library', 'zoom-in', 'photo-tool-heading'];
      return ids.map(id => {
        const element = document.getElementById(id), rect = element.getBoundingClientRect();
        return {id, unobscured:element.contains(document.elementFromPoint(rect.x+rect.width/2, rect.y+rect.height/2))};
      });
    });
    assert.ok(overlayClipped.every(value => value.unobscured), `crop overlay must stay inside the photo stage: ${JSON.stringify(overlayClipped)}`);
    if (process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-crop-zoom.png')});
    await page.locator('#navigator-fit').evaluate(button => button.click());
    console.log('Crop paper ratios, orientation, history, minimum size and overlay clipping checks passed.');
    await page.waitForFunction(()=>Math.abs(document.querySelector('#develop-overlay').getBoundingClientRect().width-document.querySelector('#full-image').getBoundingClientRect().width)<1);
    let overlay=await page.locator('#develop-overlay').boundingBox();
    await page.mouse.move(overlay.x+overlay.width*.1,overlay.y+overlay.height*.1);await page.mouse.down();
    await page.mouse.move(overlay.x+overlay.width*.75,overlay.y+overlay.height*.8,{steps:8});await page.mouse.up();
    await page.waitForFunction(()=>Number(document.querySelector('#develop-crop-2-value').value)<80);
    await page.click('#tool-done');
    await page.locator('.develop-toolstrip [data-tool="brush"]').click();
    await page.click('#mask-new');
    await page.click('[data-mask-type="linear"]');
    assert.equal(await page.locator('.mask-panel').isVisible(), true);
    assert.equal(await page.locator('#crop-options').isVisible(), false, 'crop controls must be hidden for masks');
    assert.equal(await page.locator('#rotation-options').isVisible(), false, 'rotation controls must be hidden for masks');
    overlay=await page.locator('#develop-overlay').boundingBox();
    await page.mouse.move(overlay.x+overlay.width*.5,overlay.y+overlay.height*.1);await page.mouse.down();
    await page.mouse.move(overlay.x+overlay.width*.5,overlay.y+overlay.height*.6,{steps:8});await page.mouse.up();
    await page.waitForFunction(()=>document.querySelectorAll('.mask-layer').length===1);
    await page.locator('.develop-toolstrip [data-tool="brush"]').click();
    await page.fill('#local-exposure-value','1.25');
    await page.locator('#develop-exposure').dispatchEvent('input');
    assert.equal(await page.inputValue('#local-exposure-value'), '1.25', 'control synchronization must preserve an uncommitted local number');
    await page.locator('#local-exposure-value').blur();
    await page.fill('#local-saturation-value', '300');
    await page.locator('#local-saturation-value').blur();
    await page.locator('#mask-amount').scrollIntoViewIfNeeded();
    const maskAmount = await page.locator('#mask-amount').boundingBox();
    await page.mouse.move(maskAmount.x + maskAmount.width * .6, maskAmount.y + maskAmount.height / 2);
    await page.mouse.down();
    await page.waitForFunction(() => calls.filter(c => c.command === 'develop_preview').at(-1)?.args.bound === 0);
    await page.mouse.up();
    await page.click('#mask-done');await page.click('#develop-save');
    await page.waitForFunction(()=>document.querySelector('#develop-save').disabled);
    const localRecipe=await page.evaluate(async()=> (await __TAURI__.core.invoke('get_develop',{id:1})).settings);
    assert.equal(localRecipe.masks[0].adjustments.exposure,1.25);
    assert.equal(localRecipe.masks[0].adjustments.saturation,300);
    assert.ok(localRecipe.masks[0].amount > .5 && localRecipe.masks[0].amount < .7);
    assert.ok(localRecipe.crop[2]<.8);
    await page.locator('summary').filter({hasText:/^プリセット・同期$/}).click();
    await page.fill('#recipe-name','空の仕上げ');await page.click('#preset-save');
    await page.waitForFunction(()=>document.querySelector('#user-presets').options.length===2);
    await page.locator('summary').filter({hasText:/^比較・スナップショット$/}).click();
    await page.click('#snapshot-save');await page.waitForFunction(()=>document.querySelector('#snapshot-list').options.length===2);
    await page.selectOption('#snapshot-list',{label:'空の仕上げ'});await page.click('#snapshot-compare');
    await page.waitForFunction(()=>!document.querySelector('#compare-image').hidden);
    const comparison = await page.evaluate(() => {
      const main = document.querySelector('#full-image').getBoundingClientRect();
      const before = document.querySelector('#compare-image').getBoundingClientRect();
      const area = document.querySelector('#image-area').getBoundingClientRect();
      return { main:{x:main.x,width:main.width,height:main.height}, before:{x:before.x,width:before.width,height:before.height}, area:{x:area.x,width:area.width} };
    });
    assert.ok(Math.abs(comparison.main.width-comparison.before.width)<1 && Math.abs(comparison.main.height-comparison.before.height)<1, 'matching before/after images must use the same scale');
    assert.ok(comparison.main.width+comparison.before.width+12 <= comparison.area.width-48, 'comparison must fit both images with padding');
    await page.selectOption('#compare-mode','');
    await page.locator('.film-tile').filter({has:page.locator('.film-number', {hasText:/^2$/})}).click({modifiers:['Meta']});
    assert.match(await page.textContent('#selection-count'),/2枚/);
    await page.click('#develop-sync');
    await page.waitForFunction(()=>calls.some(call=>call.command==='develop_workflow'&&call.args.request.action==='apply'));
    const synced=await page.evaluate(async()=> (await __TAURI__.core.invoke('get_develop',{id:2})).settings);
    assert.deepEqual(synced.masks,localRecipe.masks);
    await page.locator('.film-tile').filter({has:page.locator('.film-number', {hasText:/^1$/})}).click({button:'right'});
    assert.match(await page.textContent('#selection-count'), /2枚/, 'right click preserves multiple selection');
    await page.click('#photo-export');
    await page.fill('#export-quality','85');await page.fill('#export-long-edge','2048');
    await page.fill('#export-max-kb','500');await page.fill('#export-naming','trip-{seq}-{name}');
    await page.selectOption('#export-border', 'white');
    await page.selectOption('#export-border-ratio', '0.8');
    if(process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-export.png')});
    const exportViewport = page.viewportSize();
    await page.setViewportSize({width:900,height:600});
    await page.locator('#export-start').scrollIntoViewIfNeeded();
    assert.ok(await page.locator('#export-dialog').evaluate(dialog => dialog.scrollWidth <= dialog.clientWidth), 'export dialog must fit the minimum window width');
    if(process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-export-small.png')});
    await page.setViewportSize(exportViewport);

    await page.click('#export-start');
    await page.waitForFunction(()=>document.querySelector('#export-result').textContent.includes('2/2'));
    const batch=await page.evaluate(()=>calls.filter(call=>call.command==='export_develop_batch').at(-1).args.request);
    assert.deepEqual(batch.ids,[1,2]);assert.equal(batch.options.long_edge,2048);
    assert.equal(batch.options.quality,85);assert.equal(batch.options.max_bytes,512000);
    assert.deepEqual(batch.options.border, {color:'white', aspect_ratio:0.8});
    assert.equal(batch.naming,'trip-{seq}-{name}');assert.equal(batch.folder,'/exports');
    await page.fill('#export-quality','');
    await page.selectOption('#export-format','png');
    await page.selectOption('#export-border', 'black');
    await page.selectOption('#export-border-ratio', '1');
    assert.equal(await page.locator('#export-quality').isEnabled(), false);
    await page.click('#export-start');
    await page.waitForFunction(() => document.querySelector('#export-result').textContent.includes('2/2'));
    assert.equal(await page.evaluate(() => calls.filter(c=>c.command==='export_develop_batch').at(-1).args.request.options.quality), 92, 'lossless formats ignore the disabled JPEG quality input');
    assert.deepEqual(await page.evaluate(() => calls.filter(c=>c.command==='export_develop_batch').at(-1).args.request.options.border), {color:'black', aspect_ratio:1});
    await page.selectOption('#export-border', 'none');

    await page.selectOption('#export-format','tiff');
    assert.equal(await page.locator('#export-quality').isEnabled(), false);
    await page.selectOption('#export-format','jpeg');
    assert.equal(await page.locator('#export-quality').isEnabled(), true);
    await page.fill('#export-quality','85');
    await page.evaluate(() => { window.holdBatchExport = true; });
    await page.click('#export-start');
    await page.waitForFunction(() => Boolean(window.releaseBatchExport));
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('#export-dialog').isVisible(), true, 'running export cannot be dismissed');
    const closesBeforeExport = await page.evaluate(() => calls.filter(call => call.command === 'close_window').length);
    await page.evaluate(() => window.listeners['close-requested']());
    assert.equal(await page.evaluate(() => calls.filter(call => call.command === 'close_window').length), closesBeforeExport);
    await page.click('#export-cancel');
    await page.waitForFunction(() => document.querySelector('#export-result').textContent.includes('（中止）'));
    await page.evaluate(() => { window.holdBatchExport = false; window.failBatchExport = true; });
    await page.click('#export-start');
    await page.waitForFunction(() => document.querySelector('#export-result').textContent.includes('export failed'));
    assert.equal(await page.locator('#export-close').isEnabled(), true);
    await page.evaluate(() => { window.failBatchExport = false; });
    await page.click('#export-close');
    await page.click('#module-library');
    await page.locator('.film-tile').filter({has:page.locator('.film-number', {hasText:/^1$/})}).click({button:'right'});await page.click('#photo-export');
    assert.match(await page.textContent('#export-selection'), /2枚/);
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => !document.querySelector('#export-dialog').open && document.activeElement.matches('.film-tile, #filmstrip'));
    await page.keyboard.press('g');
    await page.locator('.tile').filter({has:page.locator('.grid-number', {hasText:/^3$/})}).click({button:'right'});await page.click('#photo-export');
    assert.match(await page.textContent('#export-selection'), /1枚/);
    await page.click('#export-start');
    await page.waitForFunction(() => document.querySelector('#export-result').textContent.includes('1/1'));
    assert.deepEqual(await page.evaluate(() => calls.filter(c=>c.command==='export_develop_batch').at(-1).args.request.ids), [3]);
    await page.click('#export-close');
    await page.locator('.tile').filter({has:page.locator('.grid-number', {hasText:/^3$/})}).focus();await page.keyboard.press('Shift+F10');
    assert.equal(await page.locator('#photo-menu').isVisible(), true);
    await page.keyboard.press('Escape');
    await page.locator('#photo-menu').waitFor({state:'hidden'});
    assert.equal(await page.locator('.tile.active-photo').evaluate(tile => tile === document.activeElement), true, 'Escape returns focus to the photo');
    await page.keyboard.press('Shift+F10');
    await page.locator('#photo-menu').waitFor({state:'visible'});
    await page.locator('.tile.active-photo').focus();
    await page.keyboard.press('Escape');
    await page.locator('#photo-menu').waitFor({state:'hidden'});
    await page.click('#module-develop');
    await page.locator('.film-tile').filter({has:page.locator('.film-number', {hasText:/^1$/})}).click();
    assert.match(await page.textContent('#selection-count'),/1枚/, 'plain click collapses a multiple selection');
    await page.selectOption('#compare-mode', 'side');
    await page.waitForFunction(() => {
      const image = document.querySelector('#compare-image');
      return !image.hidden && image.complete && image.naturalWidth > 0 && image.src.endsWith('-1600');
    });
    await page.selectOption('#compare-mode', '');
    if(process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-workflow.png')});
    // Point curves preserve arbitrary coordinates and persist a single drag as an edit.
    await page.locator('.tone-curve').scrollIntoViewIfNeeded();
    const curveBox=await page.locator('.tone-curve').boundingBox();
    await page.mouse.move(curveBox.x+curveBox.width*.5,curveBox.y+curveBox.height*.5);
    const displayBeforeCurve = await page.locator('#full-image').boundingBox();
    await page.evaluate(() => { window.gestureSources = []; window.gestureObserver = new MutationObserver(() => gestureSources.push(document.querySelector('#full-image').src)); gestureObserver.observe(document.querySelector('#full-image'), {attributes:true,attributeFilter:['src']}); });
    const savesBeforeCurve = await page.evaluate(() => calls.filter(c => c.command === 'save_develop').length);
    await page.mouse.down();
    await page.mouse.move(curveBox.x+curveBox.width*.42,curveBox.y+curveBox.height*.3,{steps:12});
    await page.waitForFunction(() => document.querySelector('#full-image').src.endsWith('-384'));
    await page.waitForTimeout(700);
    assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'save_develop').length), savesBeforeCurve, 'a held curve gesture must not autosave halfway');
    const draftDisplay = await page.locator('#full-image').boundingBox();
    assert.ok(Math.abs(draftDisplay.width - displayBeforeCurve.width) < 1 && Math.abs(draftDisplay.height - displayBeforeCurve.height) < 1, 'curve gestures must retain the displayed image size');
    await page.mouse.up();
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('-1600'));
    await page.waitForFunction(()=>document.querySelectorAll('#curve-handles circle').length===3);
    await page.click('#develop-save');
    await page.waitForFunction(()=>document.querySelector('#develop-save').disabled);
    const curveRecipe=await page.evaluate(async()=>(await __TAURI__.core.invoke('get_develop',{id:1})).settings);
    assert.equal(curveRecipe.point_curves[0].length,3);
    assert.ok(Math.abs(curveRecipe.point_curves[0][1][0]-.42)<.02);
    // A new gesture supersedes a slow original-size render without displaying its stale result.
    await page.evaluate(() => { delete window.releaseOriginalLoad; window.originalLoadFinished = false; window.holdOriginalLoad = true; });
    await page.click('#high-quality');
    await page.waitForFunction(() => typeof window.releaseOriginalLoad === 'function');
    const cancellationsBeforeCurve = await page.evaluate(()=>calls.filter(c=>c.command==='cancel_develop_preview').length);
    await page.mouse.move(curveBox.x+curveBox.width*.42,curveBox.y+curveBox.height*.3);
    await page.mouse.down();
    await page.mouse.move(curveBox.x+curveBox.width*.45,curveBox.y+curveBox.height*.28,{steps:5});
    await page.evaluate(() => {
      window.curveSources = [];
      window.curveObserver = new MutationObserver(() => curveSources.push(document.querySelector('#full-image').src));
      curveObserver.observe(document.querySelector('#full-image'), {attributes:true,attributeFilter:['src']});
      window.holdOriginalLoad = false; window.releaseOriginalLoad(); delete window.releaseOriginalLoad;
    });
    assert.equal(await page.evaluate(()=>calls.filter(c=>c.command==='cancel_develop_preview').length),cancellationsBeforeCurve + 1);
    await page.waitForFunction(() => window.originalLoadFinished && document.querySelector('#full-image').src.endsWith('-384'));
    assert.ok(await page.evaluate(() => curveSources.every(src => !src.includes('-stale'))), 'a cancelled render must not overwrite the newer original-size image');
    await page.evaluate(() => curveObserver.disconnect());
    await page.mouse.up();
    await page.waitForFunction(() => document.querySelector('#full-image').src.endsWith('-0'));
    await page.click('#develop-save');
    await page.waitForFunction(()=>document.querySelector('#develop-save').disabled);
    // Sliders use a lightweight preview during the gesture and restore original size on release.
    await page.locator('#develop-exposure').scrollIntoViewIfNeeded();
    const sliderBox = await page.locator('#develop-exposure').boundingBox();
    const sliderDisplay = await page.locator('#full-image').boundingBox();
    const exposureBeforeSlider = await page.inputValue('#develop-exposure-value');
    await page.fill('#develop-contrast-value', '21');
    await page.waitForFunction(() => calls.filter(c => c.command === 'develop_preview').at(-1)?.args.settings.contrast === 21 && document.querySelector('#full-image').src.endsWith('-0'));
    assert.equal(await page.locator('#develop-contrast-value').evaluate(input => input === document.activeElement), true, 'numeric editing must regain final quality without requiring blur');
    await page.waitForFunction(() => document.querySelector('#develop-save').disabled);
    const savesBeforeSlider = await page.evaluate(() => calls.filter(c => c.command === 'save_develop').length);
    await page.mouse.move(sliderBox.x + sliderBox.width * .5, sliderBox.y + sliderBox.height / 2);
    await page.mouse.down();
    await page.mouse.move(sliderBox.x + sliderBox.width * .7, sliderBox.y + sliderBox.height / 2, {steps:12});
    await page.waitForFunction(() => document.querySelector('#full-image').src.endsWith('-384'));
    await page.waitForTimeout(700);
    assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'save_develop').length), savesBeforeSlider, 'holding a slider must not save or rebuild library previews');
    assert.equal(await page.textContent('#develop-status'), '編集中');
    const sliderDraft = await page.locator('#full-image').boundingBox();
    assert.ok(Math.abs(sliderDraft.width - sliderDisplay.width) < 1 && Math.abs(sliderDraft.height - sliderDisplay.height) < 1, 'slider gestures retain zoom and image dimensions');
    await page.mouse.up();
    await page.waitForFunction(() => document.querySelector('#full-image').src.endsWith('-0'));
    await page.waitForFunction(() => document.querySelector('#develop-status').textContent === '保存済み');
    await page.click('#develop-undo');
    await page.waitForFunction(value => document.querySelector('#develop-exposure-value').value === value, exposureBeforeSlider);
    await page.waitForFunction(() => document.querySelector('#develop-save').disabled);
    assert.ok(await page.evaluate(() => gestureSources.some(src => src.endsWith('-384'))), 'gestures must render a lightweight preview');
    await page.evaluate(() => gestureObserver.disconnect());
    // Twelve gestures retain exactly ten undo operations, including after reopening the photo.
    const beforeUndoTest = await page.evaluate(()=>calls.length);
    await page.evaluate(()=>{
      const input=document.getElementById('develop-exposure');
      for(let i=0;i<12;i++) {
        input.value=String((-1+i*.1).toFixed(2));
        input.dispatchEvent(new Event('input', {bubbles:true})); input.dispatchEvent(new Event('change', {bubbles:true}));
      }
      document.activeElement?.blur();
    });
    await page.click('#develop-save');
    await page.waitForFunction(()=>document.querySelector('#develop-save').disabled);
    assert.equal(await page.evaluate(start=>calls.slice(start).filter(c=>c.command==='preview').length,beforeUndoTest),0,'autosave does not request library images');
    await page.click('#module-library'); await page.click('#module-develop');
    await page.waitForFunction(()=>document.querySelector('#develop-exposure-value').value==='0.1' && !document.querySelector('#develop-undo').disabled);
    await page.evaluate(()=>document.activeElement?.blur());
    for(let i=0;i<10;i++) {
      await page.keyboard.press('Control+z');
      assert.ok(Math.abs(Number(await page.inputValue('#develop-exposure-value'))-(0.0-i*.1))<.001);
    }
    assert.equal(await page.locator('#develop-undo').isDisabled(),true);
    await page.keyboard.press('Control+z');
    assert.equal(await page.inputValue('#develop-exposure-value'),'-0.9');
    await page.keyboard.press('Control+Shift+z');
    assert.equal(await page.inputValue('#develop-exposure-value'),'-0.8');
    await exposure(2);
    assert.equal(await page.locator('#develop-redo').isDisabled(),true,'a new edit discards the redo branch');
    await page.click('#develop-save');
    await page.waitForFunction(()=>document.querySelector('#develop-save').disabled);
    console.log('Ten undo operations, reopen, redo branch, and save without library requests passed.');

    await page.click('#module-library');
    await page.waitForFunction(() => !document.querySelector('#full-image').hidden && document.querySelector('#view-message').hidden);
    assert.equal(await page.locator('#photo-flags button').count(), 2);
    await page.click('#full-image');
    await page.evaluate(() => {
      window.flagUiBefore = {
        tiles:[...document.querySelectorAll('.film-tile')],
        calls:calls.length, src:document.querySelector('#full-image').src,
        left:document.querySelector('#image-area').scrollLeft,
        top:document.querySelector('#image-area').scrollTop,
        film:document.querySelector('#filmstrip').scrollLeft,
        title:document.querySelector('#view-title').textContent,
      };
    });
    for (const [flag, active] of [[1,true],[1,false],[-1,true],[-1,false]]) {
      await page.click(`#photo-flags [data-flag="${flag}"]`);
      await page.waitForFunction(({flag, active}) => document.querySelector(`#photo-flags [data-flag="${flag}"]`).getAttribute('aria-pressed') === String(active), {flag, active});
      const tile = page.locator('.film-tile.active-photo');
      assert.equal(await tile.locator('.photo-flag').count(), active ? 1 : 0);
      if (active) assert.equal(await tile.locator('.photo-flag').getAttribute('aria-label'), flag === 1 ? '採用' : '除外');
      assert.equal(await tile.locator('.thumb').evaluate(node => getComputedStyle(node).opacity), flag === -1 && active ? '0.35' : '1');
    }
    assert.equal(await page.evaluate(() => {
      const before = flagUiBefore, area = document.querySelector('#image-area');
      return before.tiles.every(tile => tile.isConnected)
        && !calls.slice(before.calls).some(c => ['search','preview','cancel_preview','set_preview_photo'].includes(c.command))
        && before.src === document.querySelector('#full-image').src
        && before.left === area.scrollLeft && before.top === area.scrollTop
        && before.film === document.querySelector('#filmstrip').scrollLeft
        && before.title === document.querySelector('#view-title').textContent;
    }), true, 'flag toggles must keep tiles, selection, preview and zoom/pan without reloading');
    await page.click('#full-image');
    const rotationStart = await page.evaluate(async () => {
      const id = calls.filter(c => c.command === 'set_photo_attributes').at(-1).args.ids[0];
      return {id, recipe:await __TAURI__.core.invoke('get_develop',{id}), calls:calls.length};
    });
    for (const [direction, turns] of [['right',1],['left',0],['left',3],['right',0]]) {
      await page.click(`#photo-rotate-${direction}`);
      await page.waitForFunction(() => !document.querySelector('#photo-rotate-right').disabled && document.querySelector('#view-message').hidden);
      const recipe = await page.evaluate(id => __TAURI__.core.invoke('get_develop',{id}), rotationStart.id);
      assert.equal(recipe.settings.quarter_turns, turns);
      assert.deepEqual({...recipe.settings, quarter_turns:rotationStart.recipe.settings.quarter_turns}, rotationStart.recipe.settings,
        'library rotation must preserve all other adjustments');
      assert.ok(recipe.undo.length <= 10);
    }
    assert.equal(await page.evaluate(start => calls.slice(start).some(c => c.command === 'search'), rotationStart.calls), false,
      'rotation must refresh only the edited photo');
    await page.click('#module-develop');
    await page.waitForFunction(() => document.querySelector('#develop-status').textContent === '保存済み');
    await page.click('#photo-rotate-left');
    await page.waitForFunction(() => calls.filter(c=>c.command==='develop_preview').at(-1)?.args.settings.quarter_turns===3);
    await page.click('#develop-undo');
    await page.waitForFunction(() => calls.filter(c=>c.command==='develop_preview').at(-1)?.args.settings.quarter_turns===0);
    await page.click('#module-library');
    console.log('Two toggle flags preserve list/zoom; library rotation persists both directions and Develop undo works.');
    await page.keyboard.press('x');
    await page.waitForFunction(()=>calls.some(c=>c.command==='set_photo_attributes'&&c.args.flag===-1));
    if (!await page.locator('#filter-panel').isVisible()) await page.keyboard.press('\\');
    await page.click('#filter-tabs [data-mode="attribute"]');
    await page.click('#flag-filters [data-flag="-1"]');
    await page.waitForFunction(()=>calls.filter(c=>c.command==='search').at(-1).args.query.flags.includes(-1));
    await page.waitForFunction(() => document.querySelector('#photo-flags [data-flag="-1"]').getAttribute('aria-pressed') === 'true');
    const rejectedTitle = await page.textContent('#view-title');
    await page.click('#photo-flags [data-flag="-1"]');
    await page.waitForFunction(title => document.querySelector('#view-title').textContent !== title, rejectedTitle);
    await page.selectOption('#rating-mode','lte');
    assert.equal(await page.inputValue('#rating-mode'),'lte');
    await page.click('#filter-tabs [data-mode="none"]');
    await page.click('#grid-view');await page.click('#filter-tabs [data-mode="metadata"]');
    await page.waitForFunction(()=>document.querySelectorAll('.metadata-column .facet-row').length>=8);
    assert.equal(await page.locator('.metadata-column').count(),4);
    const indexingScope = await page.evaluate(() => calls.filter(c => c.command === 'index_metadata_batch').at(-1).args.query);
    const searchScope = await page.evaluate(() => calls.filter(c => c.command === 'search').at(-1).args.query);
    assert.deepEqual(indexingScope, {root_id:searchScope.root_id, folder:searchScope.folder});
    await page.locator('[data-folder="20261020撮影"] .folder-label').click();
    await page.waitForFunction(() => calls.filter(c => c.command === 'index_metadata_batch').at(-1).args.query.folder === '20261020撮影');
    await page.waitForFunction(() => !document.querySelector('#metadata-index').disabled);
    await page.evaluate(() => { window.holdMetadataIndex = true; });
    await page.click('#metadata-index');
    await page.waitForFunction(() => window.releaseMetadataIndex);
    await page.locator('[data-folder="20261019撮影"] .folder-label').click();
    await page.waitForFunction(() => calls.some(c => c.command === 'cancel_metadata_index') &&
      calls.filter(c => c.command === 'index_metadata_batch').at(-1).args.query.folder === '20261019撮影' &&
      !document.querySelector('#metadata-index').disabled);
    await page.getByRole('button',{name:'Canon EOS M5'}).click();
    await page.waitForFunction(()=>calls.filter(c=>c.command==='search').at(-1).args.query.metadata.camera?.[0]==='Canon EOS M5');
    await page.waitForFunction(()=>!document.querySelector('#navigator-image').hidden && document.querySelector('#navigator-image').naturalWidth > 0);
    await page.setViewportSize({width:2048,height:1152});
    await page.locator('.catalog-panel').evaluate(panel => panel.open = false);
    await page.waitForFunction(() => {
      const tiles = [...document.querySelectorAll('.film-tile')].filter(tile => {
        const box = tile.getBoundingClientRect();
        return box.right > 0 && box.left < innerWidth;
      });
      return tiles.length >= 24 && tiles.every(tile => tile.querySelector('img')?.naturalWidth > 0);
    });
    // Async-decoded thumbnails can have dimensions before their pixels are painted.
    await page.evaluate(async () => {
      await Promise.all([...document.querySelectorAll('.film-tile img')].map(image => image.decode()));
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    });
    if(process.env.AE_UI_SCREENSHOT)await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-metadata.png')});
    await page.click('#filter-tabs [data-mode="none"]');
    await page.click('#module-develop');
    await page.waitForFunction(()=>!document.querySelector('#develop-controls').disabled && !document.querySelector('#full-image').hidden && document.querySelector('#full-image').naturalWidth > 0 && document.querySelector('#develop-status').textContent.includes('保存済み'));
    await page.locator('.develop-toolstrip [data-tool=""]').click();
    await page.evaluate(() => {
      document.querySelector('.develop-scroll').scrollTop = 0;
      document.querySelectorAll('#develop-left-controls details').forEach(panel => panel.open = false);
      document.querySelector('.catalog-panel').open = false;
    });
    assert.equal(await page.locator('#sidebar').isVisible(), true, 'Develop remembers its own expanded sidebar');
    await page.click('#toggle-sidebar');
    await page.click('#module-library');
    assert.equal(await page.locator('#sidebar').isVisible(), true, 'Library keeps its sidebar expanded');
    await page.click('#module-develop');
    assert.equal(await page.locator('#sidebar').isVisible(), false, 'Develop keeps its sidebar collapsed');
    await page.waitForFunction(() => !document.querySelector('#develop-controls').disabled &&
      !document.querySelector('#full-image').hidden && document.querySelector('#full-image').naturalWidth > 0 &&
      document.querySelector('#develop-status').textContent.includes('保存済み'));
    for (const width of [2048,1320,900]) {
      await page.setViewportSize({width,height:width===900?600:1152});
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
      const box = await page.locator('#viewer').boundingBox(); assert.ok(box.width > 300);
      const scroll = await page.locator('.develop-scroll').boundingBox();
      const bottom = await page.locator('.develop-bottom').boundingBox();
      assert.ok(scroll.y + scroll.height <= bottom.y + 1, 'fixed actions must never cover scrollable adjustments');
      await page.locator('.tone-curve').scrollIntoViewIfNeeded();
      const curve = await page.locator('.tone-curve').boundingBox();
      assert.ok(curve.y + curve.height <= bottom.y + 1, 'the entire curve can be reached above the footer');
      await page.evaluate(() => document.querySelector('.develop-scroll').scrollTop = 0);
      if (process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png',`-develop-${width}.png`)});
    }
    await page.click('#toggle-sidebar');
    await page.click('#module-library');
    if (!await page.locator('#filter-panel').isVisible()) await page.keyboard.press('\\');
    await page.click('#filter-tabs [data-mode="text"]');
    await page.fill('#search', 'no-matching-photos');
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '0枚の写真');
    await page.evaluate(() => {window.holdScan = true;});
    await page.click('#add-small');
    await page.waitForFunction(() => window.releaseScan);
    assert.equal(await page.locator('#add-small').isDisabled(), true);
    assert.equal(await page.evaluate(() => calls.filter(c=>c.command==='scan').at(-1).args.rootId), 2);
    await page.evaluate(() => {window.holdScan = false;window.releaseScan();});
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '42枚の写真' && !document.querySelector('#add-small').disabled);
    assert.equal(await page.inputValue('#search'), '');
    assert.match(await page.textContent('#title'), /new-photos/);
    assert.equal(await page.evaluate(() => calls.filter(c=>c.command==='search').at(-1).args.query.root_id), 2);
    await page.click('#filter-tabs [data-mode="text"]');
    await page.click('#scan');
    await page.waitForFunction(() => !document.querySelector('#scan').disabled);
    assert.equal(await page.evaluate(() => calls.filter(c=>c.command==='scan').at(-1).args.rootId), 2);
    // Import only selected folders, sequentially, and stop the queue on cancellation.
    await page.click('#add-small');
    await page.waitForFunction(() => !document.querySelector('#add-small').disabled
      && document.querySelector('#status').textContent.includes('読み込み済み42枚をスキップ'));
    assert.equal(await page.textContent('#result-count'), '42枚の写真');
    await page.evaluate(() => {window.folderChoices = [{id:2,path:'/new-photos'},{id:3,path:'/more-photos'}];});
    const batchStart = await page.evaluate(() => calls.filter(c => c.command === 'scan').length);
    await page.click('#add-small');
    await page.waitForFunction(start => calls.filter(c => c.command === 'scan').length === start + 2 && !document.querySelector('#add-small').disabled, batchStart);
    assert.deepEqual(await page.evaluate(start => calls.filter(c => c.command === 'scan').slice(start).map(c => c.args.rootId), batchStart), [2,3]);
    assert.match(await page.textContent('#roots'), /more-photos/);
    assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'search').at(-1).args.query.folder), null);
    assert.equal(await page.locator('#error').isVisible(), false, 'multi-folder import must not send a rootless folder filter');
    assert.equal(await page.locator('#roots .volume-heading').count(), 0, 'folder names must not be duplicated as volume headings');
    assert.equal(await page.locator('#roots .folder-label').filter({hasText:'more-photos'}).count(), 1);
    for (const id of [2,3,1]) {
      await page.locator(`#roots [data-root="${id}"][data-folder=""] .folder-label`).click();
      await page.waitForFunction(id => calls.filter(c => c.command === 'search').at(-1).args.query.root_id === id && document.querySelector('#result-count').textContent.endsWith('枚の写真'), id);
      assert.equal(await page.locator('#error').isVisible(), false, 'each imported folder remains independently selectable');
      assert.equal(await page.locator('#result-count').textContent(), id === 1 ? '2,000枚の写真' : '42枚の写真');
    }
    await page.locator('#roots [data-root="1"][data-folder="20261019撮影"] .folder-label').click();
    await page.waitForFunction(() => document.querySelector('#result-count').textContent === '1,200枚の写真');
    assert.equal(await page.locator('#error').isVisible(), false, 'nested folders keep their owning root');

    await page.evaluate(() => {window.holdScan = true;window.releaseScan = null;});
    const cancelStart = await page.evaluate(() => calls.filter(c => c.command === 'scan').length);
    await page.click('#add-small');
    await page.waitForFunction(() => window.releaseScan);
    await page.click('#cancel');
    await page.waitForFunction(() => !document.querySelector('#add-small').disabled);
    assert.deepEqual(await page.evaluate(start => calls.filter(c => c.command === 'scan').slice(start).map(c => c.args.rootId), cancelStart), [2]);
    await page.evaluate(() => {window.holdScan = false;window.cancelledScan = false;window.folderChoices = [];});
    await page.click('#add-small');
    await page.waitForTimeout(100);
    assert.equal(await page.evaluate(() => calls.filter(c => c.command === 'scan').length), cancelStart + 1, 'cancelled picker starts no scan');
    await page.evaluate(() => {window.catalogChoice = '/catalogs/new.openroom';});
    await page.locator('.catalog-panel').evaluate(panel => panel.open = true);
    await page.locator('#catalog-manage').evaluate(panel => panel.open = true);
    await page.click('#catalog-new');
    await page.waitForFunction(() => calls.some(call => call.command === 'restart_catalog'));
    assert.equal(await page.evaluate(() => document.querySelector('#workspace').inert), true, 'block old catalog interactions until restart');
    const callsBeforeKeys = await page.evaluate(() => calls.length);
    await page.keyboard.press('ArrowRight'); await page.keyboard.press('d');
    assert.equal(await page.evaluate(() => calls.length), callsBeforeKeys, 'keyboard must not navigate during switch');
    // MCP exercises the actual UI request gate, selection and reload paths.
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => window.listeners['mcp-request'] && !document.querySelector('#app-shell').inert);
    await page.keyboard.press('Control+,');
    assert.equal(await page.locator('#preferences-dialog').isVisible(), true);
    assert.equal(await page.locator('#preferences-title').textContent(), '環境設定');
    assert.equal(await page.locator('#mcp-title').textContent(), 'AI連携');
    await page.keyboard.press('d');
    assert.equal(await page.locator('#module-library').getAttribute('aria-current'), 'page', 'preferences blocks photo shortcuts');
    await page.click('#mcp-close');
    await page.evaluate(() => window.listeners['open-preferences']({ payload: null }));
    assert.equal(await page.locator('#preferences-dialog').isVisible(), true, 'native menu opens preferences');
    assert.equal(await page.evaluate(() => calls.some(call => call.command === 'mcp_configure')), false, 'opening preferences leaves AI configuration unchanged');
    await page.click('#mcp-close');
    assert.equal(await page.locator('#preferences-open').count(), 0);
    await page.evaluate(() => window.listeners['open-preferences']({payload:null}));
    await page.check('#mcp-enabled'); await page.click('#mcp-apply');
    await page.waitForFunction(() => document.querySelector('#mcp-status').textContent.includes('接続を受け付け'));
    if (process.env.AE_UI_SCREENSHOT) await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png','-mcp.png')});
    const blocked = await page.evaluate(() => dispatchMcp('set_view',{view:'grid'}));
    assert.ok(blocked.error, 'an open settings dialog blocks AI changes');
    await page.click('#mcp-close');
    const selection = await page.evaluate(() => dispatchMcp('select_photos',{ids:[1001,1002]}));
    assert.equal(selection.error,null); assert.equal(selection.value.active_id,1001);
    assert.deepEqual(selection.value.selected_ids,[1001,1002]);
    assert.equal(await page.locator('#ai-selection').isVisible(),true);
    const attributes = await page.evaluate(() => dispatchMcp('update_photo_attributes',{ids:[1001,1002],rating:5}));
    assert.equal(attributes.error,null); assert.equal(attributes.value.ui_updated,true);
    assert.equal(await page.evaluate(async () => (await __TAURI__.core.invoke('detail',{id:1001})).item.rating),5);
    const view = await page.evaluate(() => dispatchMcp('set_view',{view:'develop'}));
    assert.equal(view.error,null); assert.equal(view.value.view,'develop');
    await page.waitForFunction(() => !document.querySelector('#develop-controls').disabled);
    const edit = await page.evaluate(() => dispatchMcp('update_develop_settings',{id:1001,expected_revision:0,patch:{exposure:1.25}}));
    assert.equal(edit.error,null); assert.equal(edit.value.revision,1);
    assert.equal(await page.locator('#develop-exposure-value').inputValue(),'1.25');
    await page.evaluate(() => dispatchMcp('set_view',{view:'loupe'}));
    await page.click('#toggle-inspector');
    await page.fill('#annotation-memo','手動の未保存メモ');
    const unsaved = await page.evaluate(() => dispatchMcp('select_photos',{ids:[1]}));
    assert.ok(unsaved.error); assert.equal(await page.inputValue('#annotation-memo'),'手動の未保存メモ');
    assert.equal(await page.locator('#discard-dialog').isVisible(),false,'MCP must never open a discard prompt');
    const state = await page.evaluate(() => dispatchMcp('get_app_state'));
    assert.equal(state.value.unsaved.annotations,true); assert.equal(state.value.active_id,1001);
    await page.click('#annotation-save');
    await page.waitForFunction(() => !document.querySelector('#annotation-save').disabled);
    await page.evaluate(() => { const id=`test-ui-${Number(window.latestMcpId.split('-').at(-1))+1}`; window.expiredMcp.add(id); });
    const expired = await page.evaluate(() => dispatchMcp('select_photos',{ids:[1]}));
    assert.ok(expired.error); assert.equal((await page.evaluate(() => dispatchMcp('get_app_state'))).value.active_id,1001);
    await page.click('#ai-selection-clear');
    await page.waitForFunction(() => document.querySelector('#ai-selection').hidden);
    await page.goto(`http://127.0.0.1:${server.address().port}?startup=fail`);
    await page.waitForFunction(() => document.querySelector('#error').textContent.includes('Injected startup catalog failure'));
    assert.equal(await page.locator('#startup-screen').isVisible(), false, 'startup failure must reveal the error');
    assert.equal(await page.locator('#app-shell').evaluate(shell => shell.inert), false, 'startup failure must allow recovery');
    await page.waitForFunction(() => calls.some(call => call.command === 'mcp_status'));
    assert.equal(await page.evaluate(() => calls.some(call => call.command === 'mcp_ready')), false, 'failed catalog must not start MCP');
    await require('./test-raw-wb-ui.cjs')(page, `http://127.0.0.1:${server.address().port}`);
    await require('./test-crop-panel.cjs')(page, `http://127.0.0.1:${server.address().port}`);
    await require('./test-crop-pan.cjs')(page, `http://127.0.0.1:${server.address().port}`);
    await require('./test-masks.cjs')(page, `http://127.0.0.1:${server.address().port}`);
    await require('./test-histogram.cjs')(page, `http://127.0.0.1:${server.address().port}`);
    // Repeated numeric input must not cancel the slow frame or render twice on release.
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.click('#module-develop');
    await page.waitForFunction(() => document.querySelector('#full-image').src.includes('develop-'));
    const firstNumericCall = await page.evaluate(() => {
      window.holdDevelopPreview=true;
      window.previewDelay=0;
      const firstCall=calls.length;
      const input=document.getElementById('develop-exposure-value');input.focus();
      input.value=.2;input.dispatchEvent(new Event('input', {bubbles:true}));
      return firstCall;
    });
    await page.waitForFunction(() => typeof window.releaseDevelopPreview === 'function');
    await page.evaluate(() => {
      const input=document.getElementById('develop-exposure-value');
      for(const value of [.4,.6,.8]) {
        input.value=value;input.dispatchEvent(new Event('input', {bubbles:true}));
      }
      window.releaseDevelopPreview();
    });
    // Hold the next frame so the intermediate result cannot disappear between assertions.
    await page.waitForFunction(() => document.getElementById('full-image').src.includes('-0.2-1600'));
    assert.equal(await page.locator('#develop-exposure-value').inputValue(),'0.8');
    await page.waitForFunction(() => typeof window.releaseDevelopPreview === 'function');
    await page.evaluate(() => {
      const input=document.getElementById('develop-exposure-value');
      input.dispatchEvent(new Event('change', {bubbles:true}));input.blur();
      window.holdDevelopPreview=false;
      window.releaseDevelopPreview();
    });
    await page.waitForFunction(() => document.getElementById('full-image').src.includes('-0.8-1600'));
    const numericCalls = await page.evaluate(start => calls.slice(start),firstNumericCall);
    assert.equal(numericCalls.filter(c=>c.command==='cancel_develop_preview').length,0);
    assert.equal(numericCalls.filter(c=>c.command==='develop_preview').length,2,'coalesce inputs without a duplicate release render');
    await page.click('#develop-reset');
    await page.click('#develop-save');
    await page.waitForFunction(() => document.querySelector('#develop-save').disabled && !document.querySelector('.film-tile.selected .adjustment-badge'));
    await page.click('#develop-undo');
    await page.click('#develop-save');
    await page.waitForFunction(() => document.querySelector('.film-tile.selected .adjustment-badge'));
    if (process.env.AE_UI_SCREENSHOT) await page.locator('.film-tile.selected').screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png', '-badge-film.png')});
    await checkAdjustmentBadge('.film-tile.selected');
    await require('./test-copy-settings.cjs')(page);
    await require('./test-language-ui.cjs')(page, `http://127.0.0.1:${server.address().port}`);
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.locator('.film-tile[title="photo-0000.jpg"] img').waitFor();
    await page.click('#photo-flags [data-flag="1"]');
    await page.locator('.film-tile[title="photo-0000.jpg"] .flag-picked').waitFor();
    await page.click('#film-next');
    await page.click('#photo-flags [data-flag="-1"]');
    await page.locator('.film-tile[title="photo-0001.jpeg"] .flag-rejected').waitFor();
    await page.click('#film-next');
    await page.click('#rating-stars [data-rating="5"]');
    await page.waitForFunction(() => document.querySelector('.film-tile.active-photo .tile-rating').textContent === '★★★★★');
    await page.click('#grid-view');
    for (const selector of ['.tile', '.film-tile']) {
      const picked = page.locator(`${selector}[title="photo-0000.jpg"]`);
      const rejected = page.locator(`${selector}[title="photo-0001.jpeg"]`);
      const rated = page.locator(`${selector}[title="photo-0002.psd"]`);
      assert.equal(await picked.locator('.flag-picked').count(), 1);
      assert.equal(await rejected.locator('.flag-rejected .flag-cross').count(), 1);
      assert.equal(await rated.locator('.tile-rating').textContent(), '★★★★★');
      assert.match(await rated.getAttribute('aria-label'), /評価 5\/5/);
      assert.equal(await picked.evaluate(tile => {
        const flag = tile.querySelector('.photo-flag').getBoundingClientRect();
        const number = tile.querySelector('.film-number, .grid-number').getBoundingClientRect();
        const thumb = tile.querySelector('.thumb').getBoundingClientRect();
        return flag.right <= number.left && flag.bottom <= thumb.top;
      }), true, 'flag belongs before the number and above the photo');
      assert.equal(await rated.evaluate(tile => {
        const stars = tile.querySelector('.tile-rating').getBoundingClientRect();
        const thumb = tile.querySelector('.thumb').getBoundingClientRect();
        return stars.top >= thumb.bottom && stars.bottom <= tile.getBoundingClientRect().bottom;
      }), true, 'five stars must fit below the photo without clipping');
    }
    if (process.env.AE_UI_SCREENSHOT) {
      const strip = await page.locator('#filmstrip').boundingBox();
      await page.screenshot({path:process.env.AE_UI_SCREENSHOT.replace('.png', '-photo-status.png'), clip:{x:strip.x,y:strip.y,width:249,height:83}});
    }
    await require('./test-color-grading.cjs')(page);
    await require('./test-lens-correction.cjs')(page);
    await require('./test-catalog-removal.cjs')(page);
    assert.deepEqual(failures, []);
    console.log(JSON.stringify({ checks: ['MCP settings, modal and unsaved guards, off-page selection, attributes, development and expired requests', 'portrait and landscape thumbnails fit immediately with stable preview upgrades', 'new folder scans only its own root, clears filters and selects imported photos', 'LRC metadata columns, folder-scoped indexing and cancellation, rating comparisons and reject flag', 'movable smooth tone curve persisted', 'reference layout', 'loupe/grid keyboard navigation', 'virtual filmstrip and grid', 'nested folders and counts', 'file format filtering consolidated into metadata, reset and unsaved-edit protection', 'unsaved and failed-save protection', 'annotation persistence and search', 'ratings and filter', 'library uses stored 1600px previews; development requests originals', 'metadata/navigation races', 'filmstrip paging past 200 photos', 'failed thumbnail retry', '900px and 1320px layouts', 'development recipe sliders, stale renders, undo/redo, before comparison', 'development autosave failure blocks navigation and close', 'library/develop context-menu export, settings, cancellation, failures and modal guards', 'saved thumbnail race and targeted invalidation', 'crop reset bounds', 'debounced autosave and edits during an in-flight save', 'flush before photo selection, catalog snapshot and normal close', 'catalog file location and copy controls', 'persistent gesture history and nondestructive restore', 'catalog switch flush, cancellation, failure and restart isolation', 'direct crop and local mask gestures, persistent presets and snapshots, before comparison, multi-selection sync and batch export'], preview_fit_checks: previewFitChecks, cached_tiles_ms: Math.round(cacheMilliseconds), page_errors: failures }, null, 2));
  } finally {
    if (browser) await browser.close();
    server.close();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
