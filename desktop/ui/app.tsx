import { createView } from './components/view';
import { PhotoTile, FolderBranch, Properties, type FolderNode, type ThumbnailLoader } from './components/Library';
import type { ReactNode, CSSProperties } from 'react';
import type { CatalogRemoval, Commands } from './backend';
import type { Detail, Photo, Root, Folder, Cursor, Preview, PhotoMetadata, ViewMode, Module, Focus, Query, UiState, PreviewProgress, ScanProgress, Recipe } from './models';
interface Anchor extends Focus { clientX: number; clientY: number }
interface ThumbnailJob { item: Photo; key: string; token: number; alive(): boolean; complete(result: Preview): void; failed(error: unknown): void; repair: boolean }
import { element as $ } from './dom';
import { t, language, metadataValue, errorText } from './i18n';
import { createLibraryFilters } from "./library-filters";
import { createDevelop } from "./develop";
import { createPhotoExport } from "./components/PhotoExport";
import { startMcp } from "./mcp";
import { initSidebar } from "./sidebar";
import { createReview } from "./review";
import { invoke as nativeInvoke, convertFileSrc, listen } from './backend';
function invoke<K extends keyof Commands>(name: K, ...args: Commands[K]['args'] extends undefined ? [] : [Commands[K]['args']]): Promise<Commands[K]['result']> {
  return nativeInvoke(name, ...args).catch(error => Promise.reject(errorText(error)));
}

const viewport = $('viewport');
const grid = $('grid');
const filmstrip = $('filmstrip');
const filmTrack = $('filmstrip-track');
const FILM_TILE_WIDTH = 83;
const gridView = createView(grid), filmView = createView(filmTrack), foldersView = createView($('roots'));
const detailViews = new Map<HTMLElement, ReturnType<typeof createView>>();
let items: Photo[] = [], roots: Root[] = [], folders: Folder[] = [];
let cursor: Cursor | null = null, hasMore = false, loading = false, scanning = false;
let rootId: number | null = null, folderPath: string | null = null, selected = -1;
let aiIds: number[] = [], developSelectionTask = Promise.resolve();
const selectedPhotos = new Set<number>();
const adjustedPhotos = new Set<number>();
const adjustmentRequests = new Map<number, Promise<Recipe>>();
let viewMode: ViewMode = 'loupe', zoom = 0, currentPreview: Preview | null = null;
let zoomAnchor: Anchor | null = null;
let pendingPhotoAnchor: Anchor | null = null;
// Each module keeps its own panel layout during the session.
const sidebarVisible = { library: true, develop: false };
let epoch = 0, selectionEpoch = 0, viewEpoch = 0, qualityEpoch = 0;
let metadataDirty = false, metadataReady = false, saving = false;
let appliedFilters: ReturnType<typeof filterState> | null = null, searchTimer: ReturnType<typeof setTimeout> | undefined, drawFrame = 0;
const thumbnailRequests = new Map<string, Promise<Preview>>(), readyThumbnails = new Map<string, Preview>();
const largePreviews = new Map<string, Preview>(), photoMetadata = new Map<string, PhotoMetadata>();
const gridTiles = new Map<number, HTMLButtonElement>(), filmTiles = new Map<number, HTMLButtonElement>();
const thumbnailJobs: ThumbnailJob[] = [];
let thumbnailWorkers = 0;
let scanViewPending = false;
let scanCancelled = false;
const developVersions = new Map<number, number>();
const itemKey = (item: Photo) => `${item.id}:${item.modified}:${item.size}:${developVersions.get(item.id) || 0}`;
const bytes = (size: number) => size >= 1048576 ? `${(size / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(size / 1024))} KB`;

const libraryFilters = createLibraryFilters({invoke, query, refresh, error:showError, resize:scheduleDraw});
async function setPhotoAttributes(attributes: Partial<Pick<Photo, 'rating' | 'flag' | 'label'>>) {
  const item=items[selected];
  if (!item || !await allowChange()) return;
  try {
    const ids=review.active ? [item.id] : selectedPhotos.size?[...selectedPhotos]:[item.id];
    await invoke('set_photo_attributes',{ids,rating:null,flag:null,label:null,...attributes});
    for (const photo of items) if(ids.includes(photo.id)) Object.assign(photo,attributes);
    draw(); updateRatingStars(); updatePhotoAttributes(); syncReview();
    const filters = query();
    if ((attributes.flag != null && filters.flags.length)
      || (attributes.label != null && filters.labels.length)) await refresh(item.id);
    else libraryFilters.refresh();
  } catch (failure) {showError(failure);}
}
const develop = createDevelop({
  invoke, error: showError,
  photoInfo: photoJob,
  selectedIds:()=>selectedPhotos.size?[...selectedPhotos]:items[selected]?[items[selected].id]:[],
  compare(result,mode){
    const image=$('compare-image');
    image.hidden=!result; $('image-area').dataset.compare=result?mode:'';
    if(result){image.width=result.info.width; image.height=result.info.height; image.src=result.inline_image ?? convertFileSrc(result.path); zoom=0;}
    applyZoom();
  },
  show(result) {
    currentPreview = { path: result.path, inline_image: result.inline_image, width: result.info.width, height: result.info.height,
      render_width: result.info.width, render_height: result.info.height, bound: 1600 };
    const image = $('full-image');
    image.alt = items[selected]?.name || t("現像プレビュー");
    image.onload = () => { $('view-message').hidden = true; };
    image.onerror = () => { $('view-message').textContent = t("現像画像を読み込めません"); $('view-message').hidden = false; };
    setPreviewImage(currentPreview);
    $('navigator-image').src = result.inline_image ?? convertFileSrc(result.path); $('navigator-image').hidden = false;
  },
  saved(id, refreshPreview = true) {
    $('error').hidden = true;
    updateAdjustmentBadge(id);
    if (refreshPreview) invalidatePhotoPreview(id);
  },
});
function updateAdjustmentBadge(id: number) {
  const token = epoch;
  const request = invoke('get_develop', {id});
  adjustmentRequests.set(id, request);
  request.then(({settings, defaults}) => {
    if (token !== epoch || adjustmentRequests.get(id) !== request) return;
    const adjusted = (Object.keys(defaults) as (keyof typeof defaults)[])
      .some(key => JSON.stringify(settings[key]) !== JSON.stringify(defaults[key]));
    if (adjusted) adjustedPhotos.add(id); else adjustedPhotos.delete(id);
    draw();
  }).catch(failure => { if (token === epoch) showError(failure); }).finally(() => {
    if (adjustmentRequests.get(id) === request) adjustmentRequests.delete(id);
  });
}
function invalidatePhotoPreview(id: number) {
  // In-flight results keep their old key and cannot replace the newly saved recipe.
  developVersions.set(id, (developVersions.get(id) || 0) + 1);
  for (const cache of [thumbnailRequests, readyThumbnails, largePreviews]) {
    for (const key of cache.keys()) if (key.startsWith(`${id}:`)) cache.delete(key);
  }
  scheduleDraw();
}
let rotating = false;
async function rotatePhoto(turns: number) {
  const item = items[selected];
  if (!item || rotating || !await allowChange()) return;
  if (develop.active) { develop.rotate(turns); return; }
  rotating = true;
  for (const button of $('photo-rotation').querySelectorAll('button')) button.disabled = true;
  let saved = false;
  try {
    const recipe = await invoke('get_develop', {id:item.id});
    const settings = structuredClone(recipe.settings);
    settings.quarter_turns = (settings.quarter_turns + turns + 4) % 4;
    await invoke('save_develop', {id:item.id, settings, expectedRevision:recipe.revision,
      steps:[{settings, label:turns === 1 ? t("時計回りに90度回転") : t("反時計回りに90度回転")}],
      undo:[...(recipe.undo || []), recipe.settings].slice(-10)});
    saved = true;
    updateAdjustmentBadge(item.id);
    await invoke('refresh_library_preview', {id:item.id});
    invalidatePhotoPreview(item.id);
    if (review.active) syncReview();
    if (items[selected]?.id === item.id && !develop.active) {
      updateSelection();
      if (pendingPhotoAnchor) {
        const {x, y} = pendingPhotoAnchor;
        Object.assign(pendingPhotoAnchor, turns === 1 ? {x:1-y, y:x} : {x:y, y:1-x});
      }
      await displayPhoto();
    }
  } catch (failure) {
    showError(saved ? t("回転は保存済みですが、表示を更新できませんでした。{0}", failure) : failure);
  } finally {
    rotating = false;
    for (const button of $('photo-rotation').querySelectorAll('button')) button.disabled = false;
  }
}
const photoExport = createPhotoExport({ prepare: allowChange, error: showError, remove: removeFromCatalog });
const review = createReview({
  preview: largeJob, src: convertFileSrc, key: itemKey,
  activate: id => selectPhoto(items.findIndex(item => item.id === id), { reviewFocus: true }),
  async remove(id) {
    if (selectedPhotos.size <= 1 || !await allowChange()) return;
    selectedPhotos.delete(id);
    if (items[selected]?.id === id) selected = items.findIndex(item => selectedPhotos.has(item.id));
    updateSelection(); syncReview(); draw();
  },
  menu(id, event) { photoMenu(items.findIndex(item => item.id === id), event).catch(showError); },
  next: id => navigate(1, id), zoomed: updateZoomControls,
});
function syncReview() {
  if (review.active) {
    review.update(items.filter(item => selectedPhotos.has(item.id)), items[selected]?.id);
    updateNavigation();
  }
}

async function photoMenu(index: number, event: { preventDefault(): void; currentTarget: EventTarget | null; clientX?: number; clientY?: number }) {
  event.preventDefault();
  if (photoExport.busy || !items[index]) return;
  const tile = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
  const rect = tile?.getBoundingClientRect();
  if (!selectedPhotos.has(items[index].id) && !await selectPhoto(index)) return;
  const ids = selectedPhotos.size ? [...selectedPhotos] : [items[index].id];
  photoExport.menu({ kind: 'photos', ids }, (event.clientX ?? 0) || rect?.left || 0, (event.clientY ?? 0) || rect?.bottom || 0, tile ?? grid);
}
async function removeFromCatalog(selection: CatalogRemoval) {
  if (!await allowChange()) return;
  const target = selection.kind === 'photos' ? t('{0}枚の写真', selection.ids.length) : t('フォルダー「{0}」内のすべての写真（サブフォルダーを含む）', selection.path || roots.find(root => root.id === selection.root_id)?.path || '');
  confirmingChange = true;
  try {
    if (!await confirmDiscard(t('{0}をカタログから削除します。原本ファイルは削除しません。プレビュー、現像設定、編集履歴、評価などの登録情報は削除され、元に戻せません。', target), t('カタログから削除'), t('カタログから削除'))) return;
  } finally { confirmingChange = false; }
  document.body.dataset.catalogBusy = 'true';
  $('app-shell').inert = true;
  ++epoch; ++selectionEpoch; ++viewEpoch; ++qualityEpoch;
  clearTimeout(searchTimer);
  thumbnailJobs.length = 0;
  let failure: unknown, count: number | undefined;
  try {
    await developSelectionTask;
    await develop.select(null);
    await invoke('set_develop_prefetch', { ids: [] });
    count = await invoke('remove_from_catalog', { selection });
    if (selection.kind === 'folder') {
      rootId = null; folderPath = null;
      $('title').textContent = t('すべての写真');
    }
    aiIds = []; $('ai-selection').hidden = true;
  } catch (error) { failure = error; }
  finally {
    readyThumbnails.clear(); thumbnailRequests.clear(); largePreviews.clear(); photoMetadata.clear();
    delete document.body.dataset.catalogBusy;
    $('app-shell').inert = false;
  }
  await loadFolders();
  await refresh(null, true);
  if (failure) throw failure;
  $('status').textContent = t('{0}枚をカタログから削除しました。原本は残しています。', count ?? 0);
}
async function setModule(name: Module) {
  const isDevelop = name === 'develop';
  if (develop.active === isDevelop || !await allowChange()) return;
  $('error').hidden = true;
  if (!isDevelop && items[selected]) {
    confirmingChange = true;
    $('app-shell').inert = true;
    $('status').textContent = t("ライブラリのプレビューを更新中…");
    try {
      // Leaving Develop ends this edit session, even when the photo stays selected.
      await invoke('refresh_library_preview', {id:items[selected].id});
      invalidatePhotoPreview(items[selected].id);
    } catch (failure) {
      showError(failure);
      return;
    } finally {
      confirmingChange = false;
      $('app-shell').inert = false;
      $('status').textContent = t("{0}枚を読み込み済み", items.length);
    }
  }
  sidebarVisible[develop.active ? 'develop' : 'library'] = !$('sidebar').hidden;
  $('sidebar').hidden = !sidebarVisible[name];
  updatePanelButton('sidebar');
  ++viewEpoch; ++qualityEpoch;
  invoke('cancel_preview').catch(showError);
  $('full-image').hidden = true;
  $('library-inspector').hidden = true;
  document.body.classList.toggle('develop-mode', isDevelop);
  for (const module of ['library', 'develop']) {
    const button = $('module-' + module);
    button.classList.toggle('active', module === name);
    if (module === name) button.setAttribute('aria-current', 'page'); else button.removeAttribute('aria-current');
  }
  document.title = 'OpenRoom — ' + (isDevelop ? t("現像") : t("ライブラリ"));
  $('high-quality').textContent = isDevelop ? t("原寸表示") : t("現像で原本を表示");
  $('high-quality').title = isDevelop ? t("原本由来の画像を原寸で表示") : t("現像画面を開いて原本を読み込む");
  if (isDevelop) review.show('', [], null);
  await develop.activate(isDevelop, items[selected] || null);
  updateDevelopPrefetch();
  updatePanelButton('develop-panel');
  await setViewMode('loupe');
}
$('module-library').onclick = () => setModule('library');
$('module-develop').onclick = () => setModule('develop');
$('catalog-location').onclick = () => invoke('reveal_catalog').catch(showError);
$('catalog-copy').onclick = async () => {
  if (scanning || !await allowChange()) return;
  $('catalog-copy').disabled = true;
  try {
    const path = await invoke('save_catalog_copy');
    if (path) $('status').textContent = t("カタログのコピーを保存しました：{0}", path);
  } catch (failure) { showError(failure); }
  finally { $('catalog-copy').disabled = false; }
};

async function switchCatalog(create: boolean) {
  if (scanning || document.body.dataset.catalogBusy) return;
  if (!await allowChange()) return;
  document.body.dataset.catalogBusy = 'true';
  $('workspace').inert = true; $('module-bar').inert = true; $('filmstrip').inert = true;
  $('catalog-setup-new').disabled = true; $('catalog-setup-open').disabled = true;
  $('catalog-setup-error').textContent = '';
  let restarting = false;
  try {
    const path = await invoke('choose_catalog', { create });
    if (path) {
      $('status').textContent = t("カタログを切り替えています：{0}", path);
      await invoke('restart_catalog'); restarting = true;
    }
  } catch (failure) {
    showError(failure);
    $('catalog-setup-error').textContent = String(failure);
  }
  finally {
    if (!restarting) {
      delete document.body.dataset.catalogBusy;
      $('workspace').inert = false; $('module-bar').inert = false; $('filmstrip').inert = false;
      $('catalog-setup-new').disabled = false; $('catalog-setup-open').disabled = false;
    }
  }
}
$('catalog-open').onclick = () => switchCatalog(false);
$('catalog-new').onclick = () => switchCatalog(true);
$('catalog-setup-new').onclick = () => switchCatalog(true);
$('catalog-setup-open').onclick = () => switchCatalog(false);

function remember<K, V>(map: Map<K, V>, key: K, value: V, limit: number) {
  map.set(key, value);
  if (map.size > limit) map.delete(map.keys().next().value!);
  return value;
}
function showError(error: unknown) {
  $('error').textContent = String(error);
  $('error').hidden = false;
}
function confirmDiscard(message: string, title = t('未保存の変更'), action = t('変更を破棄')) {
  const dialog = $('discard-dialog');
  dialog.querySelector('h2')!.textContent = title;
  dialog.querySelector('button[value="discard"]')!.textContent = action;
  $('discard-message').textContent = message;
  dialog.returnValue = 'cancel';
  return new Promise<boolean>(resolve => {
    dialog.addEventListener('close', () => resolve(dialog.returnValue === 'discard'), { once: true });
    dialog.showModal();
  });
}
let confirmingChange = false;
async function allowChange() {
  if (photoExport.busy || saving || rotating || confirmingChange || document.body.dataset.catalogBusy) return false;
  confirmingChange = true;
  try {
    if (metadataDirty && !await confirmDiscard(t("未保存のタグ・評価・メモを破棄しますか？"))) return false;
    if (!await develop.allowChange()) return false;
    metadataDirty = false;
    return true;
  } catch (error) { showError(error); return false; }
  finally { confirmingChange = false; }
}

function query(): Query {
  return {
    ...libraryFilters.query(),
    ids: aiIds,
    text: $('search').value,
    root_id: rootId,
    folder: folderPath,
    rating: $('rating-filter').value === '' ? null : Number($('rating-filter').value),
    sort: $('sort').value,
    descending: $('descending').getAttribute('aria-pressed') === 'true',
  };
}
function filterState() {
  return { library:libraryFilters.snapshot(), rootId, folderPath, aiIds: [...aiIds], text: $('search').value,
    rating: $('rating-filter').value, sort: $('sort').value,
    descending: $('descending').getAttribute('aria-pressed'), title: $('title').textContent };
}
function restoreFilters() {
  if (!appliedFilters) return;
  ({ rootId, folderPath } = appliedFilters);
  aiIds = appliedFilters.aiIds || [];
  $('ai-selection').hidden = !aiIds.length;
  for (const [id, key] of [['search', 'text'], ['rating-filter', 'rating'], ['sort', 'sort']] as const) $(id).value = appliedFilters[key];
  $('descending').setAttribute('aria-pressed', appliedFilters.descending ?? 'false');
  $('descending').textContent = appliedFilters.descending === 'true' ? '↓' : '↑';
  $('title').textContent = appliedFilters.title;
  libraryFilters.restore(appliedFilters.library);
  updateFolderSelection();
}
function updateEmpty() {
  const empty = !items.length && !loading;
  $('empty').hidden = !empty;
  $('empty-title').textContent = roots.length ? t("該当する写真がありません") : t("写真フォルダを読み込んでください");
  $('empty-description').textContent = roots.length ? t("検索条件を変えるか、フォルダを再読み込みしてください。") : t("原本を移動せず、写真をカタログで管理できます。");
  $('empty-add').hidden = roots.length > 0;
}
async function refresh(keepId: number | null = null, throwErrors = false) {
  if (!await allowChange()) { restoreFilters(); return; }
  appliedFilters = filterState();
  const retainedSelection = keepId && review.active ? [...selectedPhotos] : [];
  const token = ++epoch;
  ++viewEpoch;
  invoke('cancel_preview').catch(showError);
  items = [];
  adjustedPhotos.clear(); adjustmentRequests.clear();
  cursor = null;
  hasMore = true;
  loading = false;
  selected = -1; selectedPhotos.clear();
  viewport.scrollTop = 0;
  filmstrip.scrollLeft = 0;
  clearTiles();
  thumbnailJobs.length = 0;
  updateSelection();
  $('error').hidden = true;
  $('result-count').textContent = t("検索中…");
  libraryFilters.refresh();
  const requestedQuery = query();
  await more(token, throwErrors);
  if (keepId && !items.some(item => item.id === keepId) && hasMore) {
    const scope = query();
    const matches = (!scope.ids.length || scope.ids.includes(keepId)) && await invoke('count', { query: { ...scope, ids: [keepId] } });
    while (matches && hasMore && token === epoch && !items.some(item => item.id === keepId)) await more(token, throwErrors);
  }
  if (token !== epoch) return;
  if (retainedSelection.length) {
    const scope = query();
    const ids = retainedSelection.filter(id => !scope.ids.length || scope.ids.includes(id));
    const count = ids.length ? await invoke('count', { query: { ...scope, ids } }) : 0;
    while (token === epoch && hasMore && items.filter(item => ids.includes(item.id)).length < count) await more(token, throwErrors);
    if (token !== epoch) return;
    for (const item of items) if (ids.includes(item.id)) selectedPhotos.add(item.id);
  }
  if (items.length) {
    const kept = items.findIndex(item => item.id === keepId);
    await selectPhoto(kept >= 0 ? kept : 0);
  }
  syncReview();
  invoke('count', { query: requestedQuery }).then(count => {
    if (token !== epoch) return;
    $('result-count').textContent = t("{0}枚の写真", count.toLocaleString());
  }).catch(error => { if (token === epoch) showError(error); });
}
async function more(token = epoch, throwErrors = false) {
  if (loading || !hasMore) return;
  loading = true;
  try {
    const result = await invoke('search', { query: query(), cursor }).catch(async error => {
      if (!cursor || !String(error).includes('stale or incompatible cursor; restart pagination')) throw error;
      await restartPages(token);
      return null;
    });
    if (!result) return;
    if (token !== epoch) return;
    items.push(...result.items);
    for (const id of result.adjusted_ids) adjustedPhotos.add(id);
    cursor = result.next;
    hasMore = Boolean(cursor);
    updateDevelopPrefetch();
    $('status').textContent = t("{0}枚を読み込み済み", items.length.toLocaleString());
  } catch (error) {
    if (token === epoch) { hasMore = false; showError(error); }
    if (throwErrors) throw error;
  } finally {
    if (token === epoch) { loading = false; updateEmpty(); draw(); }
  }
}
async function restartPages(token: number) {
  const scope = query(), loadedCount = items.length;
  const replacement: Photo[] = [], adjusted = new Set<number>();
  let next: Cursor | null = null;
  // Rebuild offscreen; never mix catalog generations or discard an unsaved edit.
  do {
    const page: Commands['search']['result'] = await invoke('search', {query: scope, cursor: next});
    if (token !== epoch) return;
    replacement.push(...page.items);
    for (const id of page.adjusted_ids) adjusted.add(id);
    next = page.next;
  } while (next && replacement.length <= loadedCount);
  const activeId = items[selected]?.id;
  const kept = replacement.findIndex(item => item.id === activeId);
  if (activeId != null && kept < 0 && !await allowChange()) { hasMore = false; return; }
  if (token !== epoch) return;
  items = replacement;
  cursor = next;
  hasMore = Boolean(next);
  selected = kept;
  const ids = new Set(items.map(item => item.id));
  for (const id of selectedPhotos) if (!ids.has(id)) selectedPhotos.delete(id);
  adjustedPhotos.clear();
  for (const id of adjusted) adjustedPhotos.add(id);
  if (activeId != null && kept < 0) {
    if (items.length) await selectPhoto(0);
    else updateSelection();
  }
  updateDevelopPrefetch();
  updateNavigation();
  updateRatingStars();
  updatePhotoAttributes();
  syncReview();
  $('status').textContent = t("{0}枚を読み込み済み", items.length.toLocaleString());
}
function clearTiles() {
  gridView.render(null); filmView.render(null);
  gridTiles.clear(); filmTiles.clear();
}
function scheduleDraw() {
  cancelAnimationFrame(drawFrame);
  drawFrame = requestAnimationFrame(draw);
}
function draw() {
  if (viewMode === 'grid') drawGrid();
  if (!filmstrip.hidden) drawFilmstrip();
  pumpThumbnails();
}
const loadThumbnail: ThumbnailLoader = (item, complete, failed, alive, repair) => {
  thumbnailJobs.push({ item, key: itemKey(item), token: epoch, complete, failed, alive, repair });
  pumpThumbnails();
};
function photoTile(index: number, film: boolean, style: CSSProperties) {
  const item = items[index], tiles = film ? filmTiles : gridTiles;
  return <PhotoTile key={itemKey(item)} photo={item} index={index} film={film} selected={selectedPhotos.has(item.id)} active={selected === index}
    adjusted={adjustedPhotos.has(item.id)}
    style={style} source={convertFileSrc} load={loadThumbnail} sizeLabel={bytes(item.size)}
    element={node => { if (node) tiles.set(index, node); else tiles.delete(index); }}
    select={event => selectPhoto(index, event)} menu={event => { photoMenu(index, event).catch(showError); }}
    open={async () => { if (await selectPhoto(index)) await setViewMode('loupe'); }} />;
}
function drawGrid() {
  const size = Number($('tile-size').value);
  const columns = Math.max(1, Math.floor(viewport.clientWidth / size));
  const width = viewport.clientWidth / columns;
  const rowHeight = Math.max(170, Math.round(width));
  grid.style.setProperty('--thumb-height', `${rowHeight - 90}px`);
  grid.style.height = `${Math.ceil(items.length / columns) * rowHeight}px`;
  const first = Math.max(0, Math.floor(viewport.scrollTop / rowHeight) - 2) * columns;
  const end = Math.min(items.length, (Math.ceil((viewport.scrollTop + viewport.clientHeight) / rowHeight) + 2) * columns);
  const tiles: ReactNode[] = [];
  for (let index = first; index < end; index++) tiles.push(photoTile(index, false, {
    width: `${width}px`, height: `${rowHeight}px`, left: `${index % columns * width}px`, top: `${Math.floor(index / columns) * rowHeight}px`,
  }));
  gridView.render(tiles);
  if (hasMore && !loading && viewport.scrollTop + viewport.clientHeight > grid.offsetHeight - 500) more();
}
function drawFilmstrip() {
  filmTrack.style.width = `${items.length * FILM_TILE_WIDTH}px`;
  const first = Math.max(0, Math.floor(filmstrip.scrollLeft / FILM_TILE_WIDTH) - 2);
  const end = Math.min(items.length, Math.ceil((filmstrip.scrollLeft + filmstrip.clientWidth) / FILM_TILE_WIDTH) + 2);
  const tiles: ReactNode[] = [];
  for (let index = first; index < end; index++) tiles.push(photoTile(index, true, { left: `${index * FILM_TILE_WIDTH}px` }));
  filmView.render(tiles);
  if (hasMore && !loading && filmstrip.scrollLeft + filmstrip.clientWidth > filmTrack.offsetWidth - 350) more();
}
function ensureFilmVisible() {
  const left = selected * FILM_TILE_WIDTH;
  if (left < filmstrip.scrollLeft) filmstrip.scrollLeft = left;
  else if (left + FILM_TILE_WIDTH > filmstrip.scrollLeft + filmstrip.clientWidth) filmstrip.scrollLeft = left + FILM_TILE_WIDTH - filmstrip.clientWidth;
}
function thumbnail(item: Photo, repair = false) {
  const key = itemKey(item);
  if (repair) { thumbnailRequests.delete(key); readyThumbnails.delete(key); }
  let request = thumbnailRequests.get(key);
  if (!request) {
    request = repair
      ? invoke('invalidate_library_preview', { id: item.id, bound: 384 }).then(() => invoke('preview', { id: item.id, large: false }))
      : invoke('preview', { id: item.id, large: false });
    remember(thumbnailRequests, key, request, 600);
    request.then(result => { if (thumbnailRequests.get(key) === request) remember(readyThumbnails, key, result, 600); }).catch(() => {
      if (thumbnailRequests.get(key) === request) thumbnailRequests.delete(key);
    });
  }
  return request;
}
function pumpThumbnails() {
  while (thumbnailWorkers < 8 && thumbnailJobs.length) {
    const job = thumbnailJobs.shift()!;
    const current = () => job.token === epoch && job.alive() && job.key === itemKey(job.item);
    if (!current()) continue;
    const cached = !job.repair && readyThumbnails.get(job.key);
    if (cached) { job.complete(cached); continue; }
    thumbnailWorkers++;
    thumbnail(job.item, job.repair).then(result => { if (current()) job.complete(result); })
      .catch(error => { if (current()) job.failed(error); })
      .finally(() => { thumbnailWorkers--; pumpThumbnails(); });
  }
}

async function loadFolders() {
  [roots, folders] = await Promise.all([invoke('roots'), invoke('folders')]);
  $('catalog-count').textContent = folders.filter(folder => folder.path === '').reduce((sum, folder) => sum + folder.count, 0).toLocaleString();
  $('no-folders').hidden = roots.length > 0;
  renderFolders();
}
function renderFolders() {
  const filter = $('folder-search').value.toLocaleLowerCase();
  const branches: ReactNode[] = [];
  for (const root of roots) {
    const separator = root.path.includes('\\') ? '\\' : '/';
    const rootName = root.path.split(/[\\/]/).filter(Boolean).pop() || root.path;
    const tree: FolderNode = { path: '', name: rootName, count: 0, children: new Map() };
    for (const folder of folders.filter(folder => folder.root_id === root.id)) {
      if (!folder.path) { tree.count = folder.count; continue; }
      let node = tree;
      const segments = folder.path.split(/[\\/]/);
      for (let index = 0; index < segments.length; index++) {
        const name = segments[index];
        if (!node.children.has(name)) node.children.set(name, { path: segments.slice(0, index + 1).join(separator), name, count: 0, children: new Map() });
        node = node.children.get(name)!;
      }
      node.count = folder.count;
    }
    branches.push(<FolderBranch key={`${root.id}:${filter}`} root={root} node={tree} depth={0} filter={filter} rootId={rootId} folderPath={folderPath} choose={chooseFolder} menu={(root, node, event) => {
      event.preventDefault();
      if (document.body.dataset.catalogBusy || confirmingChange || photoExport.busy) return;
      const target = event.currentTarget as HTMLElement;
      const rect = target.getBoundingClientRect();
      photoExport.menu({ kind: 'folder', root_id: root.id, path: node.path }, 'clientX' in event ? event.clientX : rect.left, 'clientY' in event ? event.clientY : rect.bottom, target.querySelector('button.folder-label') ?? target);
    }} />);
  }
  foldersView.render(branches);
  updateFolderSelection();
}
function chooseFolder(root: Root, node: FolderNode) {
  aiIds = []; $('ai-selection').hidden = true;
  rootId = root.id; folderPath = node.path;
  $('title').textContent = t('フォルダー：{0}', node.name);
  updateFolderSelection(); refresh();
}
function updateFolderSelection() {
  $('all').classList.toggle('active', rootId === null);
  for (const row of $('roots').querySelectorAll<HTMLElement>('[data-folder]')) {
    row.classList.toggle('active', Number(row.dataset.root) === rootId && row.dataset.folder === folderPath);
  }
}
async function addFolder() {
  if (scanning) return;
  if (!await allowChange()) return;
  try {
    const roots = await invoke('choose_folder');
    const root = roots[0];
    if (!root) return;
    aiIds = []; $('ai-selection').hidden = true;
    rootId = roots.length === 1 ? root.id : null;
    folderPath = rootId === null ? null : '';
    libraryFilters.clear();
    $('folder-search').value = '';
    $('title').textContent = roots.length === 1 ? t("フォルダー：{0}", root.path.split(/[\\/]/).filter(Boolean).at(-1)) : t("すべての写真");
    await loadFolders();
    await refresh();
    await scan(roots.map(root => root.id));
  } catch (error) { showError(error); }
}
async function scan(scanRootIds: (number | null)[] = [null]) {
  if (scanning || !await allowChange()) return;
  scanning = true;
  scanCancelled = false;
  scanViewPending = true;
  $('scan').disabled = true;
  for (const id of ['add', 'add-small', 'empty-add'] as const) $(id).disabled = true;
  $('progress').hidden = false;
  $('progress-text').textContent = t("フォルダを読み込み中…");
  try {
    for (const id of scanRootIds) {
      const result = await invoke('scan', { rootId: id });
      if (result.indexed || result.missing || result.previews?.generated) {
        thumbnailRequests.clear(); readyThumbnails.clear(); largePreviews.clear(); photoMetadata.clear();
      }
      await loadFolders();
      await refresh(items[selected]?.id);
      $('status').textContent = result.cancelled ? t("読み込みを中止しました") : t("{0}枚を追加・更新／読み込み済み{1}枚をスキップ{2}", result.indexed, result.unchanged, result.previews?.generated ? t("・プレビュー {0}/{1}枚を準備{2}", result.previews.ready, result.previews.total, result.previews.retained ? t("・{0}枚は一覧のみ保持", result.previews.retained) : '') : '');
      if (result.previews?.cancelled) $('status').textContent += t("・プレビュー生成を中止しました");
      if (result.previews?.error) showError(result.previews.error);
      if (result.errors.length) showError(t("{0}件を読み込めませんでした。\n{1}", result.errors.length, result.errors.slice(0, 5).join('\n')));
      if (scanCancelled || result.cancelled || result.previews?.cancelled) break;
    }
  } catch (error) { showError(error); }
  finally {
    scanning = false;
    scanViewPending = false;
    $('scan').disabled = false;
    for (const id of ['add', 'add-small', 'empty-add'] as const) $(id).disabled = false;
    $('progress').hidden = true;
  }
}

async function selectPhoto(index: number, event: { reviewFocus?: boolean; metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean } = {}) {
  if (!items[index]) return false;
  if (review.active) {
    if (event.reviewFocus && items[selected]?.id === items[index].id) return true;
    if (!await allowChange()) return false;
    const id = items[index].id;
    if ((event.metaKey || event.ctrlKey) && selectedPhotos.has(id) && id !== items[selected]?.id) selectedPhotos.delete(id);
    else {
      if (event.shiftKey && selected >= 0) for (let n = Math.min(selected, index); n <= Math.max(selected, index); n++) selectedPhotos.add(items[n].id);
      selectedPhotos.add(id);
      selected = index;
    }
    ensureFilmVisible(); updateSelection(); syncReview(); draw();
    return true;
  }
  if (event.metaKey || event.ctrlKey) {
    if (!await develop.allowChange()) return false;
    const id = items[index].id;
    if (selectedPhotos.has(id) && index !== selected) selectedPhotos.delete(id);
    else selectedPhotos.add(id);
    $('selection-count').textContent = t("/ {0}枚選択 /", selectedPhotos.size);
    draw();
    return true;
  }
  if (index === selected && !event.shiftKey) {
    if (!await develop.allowChange()) return false;
    selectedPhotos.clear();
    selectedPhotos.add(items[index].id);
    $('selection-count').textContent = t("/ 1枚選択 /");
    draw();
    return true;
  }
  if (!await allowChange()) return false;
  if (event.shiftKey && selected >= 0) {
    for (let n = Math.min(selected, index); n <= Math.max(selected, index); n++) selectedPhotos.add(items[n].id);
  } else {
    selectedPhotos.clear();
    selectedPhotos.add(items[index].id);
  }
  selected = index;
  ensureFilmVisible();
  updateSelection();
  draw();
  if (viewMode === 'loupe') displayPhoto();
  return true;
}
function setInfo(detail: Detail) {
  const item = detail.item;
  setProperties($('selection-details'), {
    [t("ファイル名")]: item.name, [t("形式")]: item.extension.toUpperCase(), [t("容量")]: bytes(item.size),
    [t("更新日")]: new Date(item.modified / 1e6).toLocaleString(language), [t("保存場所")]: item.path,
  });
}
function setProperties(target: HTMLElement, fields: PhotoMetadata) {
  let view = detailViews.get(target);
  if (!view) { view = createView(target); detailViews.set(target, view); }
  view.render(<Properties fields={fields} />);
}
async function photoJob(item: Photo) {
  const key = itemKey(item);
  if (photoMetadata.has(key)) return photoMetadata.get(key)!;
  const token = epoch;
  const fields = await invoke('photo_metadata', { id: item.id });
  if (token === epoch) remember(photoMetadata, key, fields, 100);
  return fields;
}
function updateDevelopPrefetch() {
  const offsets = develop.active ? [0, 1, -1, 2, -2, 3, -3] : [0];
  const ids = items[selected] ? offsets.flatMap(offset => {
    const photo = items[selected + offset];
    return photo && !photo.missing ? [photo.id] : [];
  }) : [];
  // Send removals immediately; the backend delays and cancels speculative decoding.
  void invoke('set_develop_prefetch', {ids}).catch(() => {});
  if (develop.active && selected >= 0 && selected + 3 >= items.length && hasMore && !loading) void more();
}
function updateSelection() {
  pendingPhotoAnchor = capturePhotoAnchor();
  const token = ++selectionEpoch;
  const item = items[selected];
  if (item) invoke('set_preview_photo', {id:item.id}).catch(showError);
  if (develop.active) developSelectionTask = develop.select(item || null);
  updateDevelopPrefetch();
  $('selection-empty').hidden = Boolean(item);
  $('selection-content').hidden = !item;
  $('selection-count').textContent = item ? t("/ {0}枚選択 /", selectedPhotos.size || 1) : '';
  $('view-title').textContent = item?.name || '';
  updatePhotoAttributes();
  $('navigator-image').hidden = true;
  $('navigator-image').removeAttribute('src');
  $('navigator-viewport').hidden = true;
  $('navigator-empty').hidden = Boolean(item);
  setProperties($('selection-details'), {});
  setProperties($('photo-details'), {});
  metadataReady = false;
  setFormDisabled(true);
  updateRatingStars();
  updateNavigation();
  if (!item) {
    $('full-image').hidden = true;
    $('full-image').removeAttribute('src');
    $('view-message').hidden = true;
    syncReview();
    return;
  }
  $('selection-name').textContent = item.name;
  $('annotation-status').textContent = '';
  $('photo-status').textContent = t("撮影情報を読み込み中…");
  invoke('detail', { id: item.id }).then(detail => {
    if (token !== selectionEpoch) return;
    setInfo(detail);
    $('annotation-tags').value = detail.tags.join(', ');
    $('annotation-rating').value = String(detail.item.rating);
    $('annotation-memo').value = detail.memo;
    metadataReady = true;
    setFormDisabled(false);
    updateRatingStars();
  }).catch(error => { if (token === selectionEpoch) showError(error); });
  thumbnail(item).then(preview => {
    if (token !== selectionEpoch || develop.active) return;
    const image = $('navigator-image');
    image.onload = () => { if (token === selectionEpoch) image.hidden = false; };
    image.onerror = () => { if (token === selectionEpoch) image.hidden = true; };
    image.src = convertFileSrc(preview.path);
  }).catch(() => {});
  if (!$('library-inspector').hidden) loadPhotoInfo(item, token);
}
function updatePhotoAttributes() {
  const item = items[selected];
  $('photo-label').value = item?.label || '';
  for (const button of $('photo-flags').querySelectorAll('button')) {
    button.setAttribute('aria-pressed', String(Number(button.dataset.flag) === (item?.flag || 0)));
  }
}
function loadPhotoInfo(item: Photo, token = selectionEpoch) {
  photoJob(item).then(fields => {
    if (token !== selectionEpoch) return;
    setProperties($('photo-details'), fields);
    $('photo-status').textContent = Object.keys(fields).length ? '' : t("撮影情報はありません。");
  }).catch(() => { if (token === selectionEpoch) $('photo-status').textContent = t("撮影情報を取得できませんでした。"); });
}
function setFormDisabled(disabled: boolean) {
  for (const element of $('annotation-form').querySelectorAll<HTMLInputElement | HTMLTextAreaElement | HTMLButtonElement>('input,textarea,button')) element.disabled = disabled;
}
function updateRatingStars() {
  const item = items[selected];
  for (const button of $('rating-stars').querySelectorAll('button')) {
    button.classList.toggle('rated', Boolean(item) && Number(button.dataset.rating) <= item.rating);
    button.disabled = !metadataReady || saving;
    button.setAttribute('aria-pressed', String(Boolean(item) && Number(button.dataset.rating) <= item.rating));
  }
}
async function saveAnnotation(event?: Event) {
  event?.preventDefault();
  const item = items[selected];
  if (!item || !metadataReady || saving) return;
  const annotation = {
    tags: $('annotation-tags').value.split(/[,、]/).map(tag => tag.trim()).filter(Boolean),
    rating: Number($('annotation-rating').value), memo: $('annotation-memo').value,
  };
  saving = true;
  setFormDisabled(true);
  updateRatingStars();
  try {
    await invoke('annotate', { ids: [item.id], annotation });
    item.rating = annotation.rating;
    metadataDirty = false;
    $('annotation-status').textContent = t("保存しました");
    $('status').textContent = t("タグ・評価・メモを保存しました");
    syncReview(); draw();
    if ($('search').value || $('rating-filter').value !== '') {
      saving = false;
      await refresh(item.id);
    }
  } catch (error) {
    showError(error);
    $('annotation-status').textContent = t("保存できませんでした。編集内容を保持しています。");
  } finally { saving = false; setFormDisabled(!metadataReady); updateRatingStars(); }
}
function updateNavigation() {
  const index = review.comparing ? items.findIndex(item => item.id === review.candidateId) : selected;
  const previous = index - (items[index - 1]?.id === review.referenceId && review.comparing ? 2 : 1);
  const next = index + (items[index + 1]?.id === review.referenceId && review.comparing ? 2 : 1);
  $('film-previous').disabled = index < 0 || previous < 0;
  $('film-next').disabled = index < 0 || (next >= items.length && !hasMore);
}
async function setViewMode(mode: ViewMode) {
  if (develop.active && mode !== 'loupe') { await setModule('library'); if (develop.active) return; }
  if (!await allowChange()) return;
  if (mode === 'compare' || mode === 'survey') {
    if (selected >= 0) selectedPhotos.add(items[selected].id);
    if (selectedPhotos.size < 2 && items.length) {
      if (selected === items.length - 1 && hasMore) await more();
      const other = items[selected + 1] || items[selected - 1];
      if (other) selectedPhotos.add(other.id);
    }
  }
  viewMode = mode;
  $('viewport').hidden = mode !== 'grid';
  $('viewer').hidden = mode !== 'loupe';
  $('grid-tools').hidden = mode !== 'grid';
  $('loupe-tools').hidden = mode !== 'loupe' && mode !== 'compare';
  review.show(['compare', 'survey'].includes(mode) ? mode : '', items.filter(item => selectedPhotos.has(item.id)), items[selected]?.id);
  $('selection-count').textContent = items[selected] ? t("/ {0}枚選択 /", selectedPhotos.size) : '';
  for (const id of ['grid', 'loupe', 'compare', 'survey']) {
    $(id + '-view').classList.toggle('active', mode === id);
    $(id + '-view').setAttribute('aria-pressed', String(mode === id));
  }
  toggleFilters(mode === 'grid');
  updateNavigation();
  ++viewEpoch;
  invoke('cancel_preview').catch(showError);
  if (mode === 'loupe') displayPhoto();
  else if (mode === 'grid' && selected >= 0) {
    const columns = Math.max(1, Math.floor(viewport.clientWidth / Number($('tile-size').value)));
    const rowHeight = Math.max(170, Math.round(viewport.clientWidth / columns));
    viewport.scrollTop = Math.floor(selected / columns) * rowHeight;
  }
  draw();
}
async function largeJob(item: Photo, bound: number, retry = false) {
  const token = epoch;
  const key = `${itemKey(item)}:${bound}`;
  if (retry) {
    largePreviews.delete(key);
    await invoke('invalidate_library_preview', { id: item.id, bound });
  }
  const result = await invoke('preview', { id: item.id, large: true, bound });
  if (token === epoch && !result.inline_image) remember(largePreviews, key, result, 32);
  return result;
}
async function displayPhoto() {
  if (develop.active) return;
  const item = items[selected];
  if (!item || viewMode !== 'loupe') return;
  const token = ++viewEpoch;
  const image = $('full-image');
  currentPreview = null;
  const warm=largePreviews.get(`${itemKey(item)}:1600`) || readyThumbnails.get(itemKey(item));
  if(warm) setPreviewImage(warm);
  else {image.removeAttribute('src');image.hidden=true;}
  image.alt = item.name;
  $('view-message').textContent = t("プレビューを読み込み中…");
  $('view-message').hidden = Boolean(warm);
  thumbnail(item).then(preview => {
    if (token === viewEpoch && !currentPreview && !warm) setPreviewImage(preview);
  }).catch(() => {});
  try {
    await invoke('cancel_preview');
    if (token !== viewEpoch) return;
    await showLarge(item, 1600, token);
  } catch (error) {
    if (token === viewEpoch) { $('view-message').textContent = String(error); $('view-message').hidden = false; }
  }
}
async function showLarge(item: Photo, bound: number, token = viewEpoch, retried = false) {
  const quality = ++qualityEpoch;
  const result = await largeJob(item, bound);
  if (token !== viewEpoch || quality !== qualityEpoch) return;
  currentPreview = { ...result, bound };
  const image = $('full-image');
  image.onload = () => { if (token === viewEpoch) $('view-message').hidden = true; };
  image.onerror = () => {
    if (token !== viewEpoch) return;
    largePreviews.delete(`${itemKey(item)}:${bound}`);
    $('view-message').textContent = t("表示画像を読み込めません。写真を選び直してください。");
    $('view-message').hidden = false;
    if (!retried) invoke('invalidate_library_preview',{id:item.id,bound}).then(() => {
      if (token === viewEpoch) return showLarge(item,bound,token,true);
    }).catch(showError);
  };
  setPreviewImage(result);
}
async function highQuality() {
  if (develop.active) develop.highQuality();
}
function setPreviewImage(preview: Preview) {
  const image = $('full-image');
  const anchor = pendingPhotoAnchor || zoomAnchor;
  pendingPhotoAnchor = null;
  // Reserve the correct aspect ratio before decoding either preview resolution.
  image.width = preview.render_width;
  image.height = preview.render_height;
  image.src = preview.inline_image ?? convertFileSrc(preview.path);
  applyZoom();
  image.hidden = false;
  if (zoom && anchor) positionZoomAnchor(anchor);
}
function updateNavigatorViewport() {
  const frame = $('navigator-viewport');
  const image = $('full-image');
  const thumbnail = $('navigator-image');
  frame.hidden = true;
  if (!zoom || viewMode !== 'loupe' || image.hidden || thumbnail.hidden || !thumbnail.complete || !thumbnail.naturalWidth) return;
  const photo = image.getBoundingClientRect();
  const area = $('image-area');
  const viewport = area.getBoundingClientRect();
  const left = Math.max(photo.left, viewport.left + area.clientLeft);
  const top = Math.max(photo.top, viewport.top + area.clientTop);
  const right = Math.min(photo.right, viewport.left + area.clientLeft + area.clientWidth);
  const bottom = Math.min(photo.bottom, viewport.top + area.clientTop + area.clientHeight);
  if (right <= left || bottom <= top || (right - left >= photo.width && bottom - top >= photo.height)) return;
  const style = getComputedStyle(thumbnail);
  const paddingLeft = parseFloat(style.paddingLeft), paddingTop = parseFloat(style.paddingTop);
  const width = thumbnail.clientWidth - paddingLeft - parseFloat(style.paddingRight);
  const height = thumbnail.clientHeight - paddingTop - parseFloat(style.paddingBottom);
  const scale = Math.min(width / thumbnail.naturalWidth, height / thumbnail.naturalHeight);
  if (scale <= 0) return;
  const thumbnailWidth = thumbnail.naturalWidth * scale, thumbnailHeight = thumbnail.naturalHeight * scale;
  const bounds = thumbnail.getBoundingClientRect();
  const container = thumbnail.parentElement!;
  const origin = container.getBoundingClientRect();
  // Map onto the painted photo, excluding its border, padding and object-fit margins.
  const thumbnailLeft = bounds.left + thumbnail.clientLeft + paddingLeft + (width - thumbnailWidth) / 2 - origin.left - container.clientLeft;
  const thumbnailTop = bounds.top + thumbnail.clientTop + paddingTop + (height - thumbnailHeight) / 2 - origin.top - container.clientTop;
  Object.assign(frame.style, {
    left: `${thumbnailLeft + (left - photo.left) / photo.width * thumbnailWidth}px`,
    top: `${thumbnailTop + (top - photo.top) / photo.height * thumbnailHeight}px`,
    width: `${(right - left) / photo.width * thumbnailWidth}px`,
    height: `${(bottom - top) / photo.height * thumbnailHeight}px`,
  });
  frame.hidden = false;
}
function applyZoom() {
  zoomAnchor = null;
  const image = $('full-image');
  const area = $('image-area');
  const style = getComputedStyle(area);
  let width = area.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
  const height = area.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
  if (area.dataset.compare === 'side') width = (width - parseFloat(style.columnGap)) / 2;
  // Leave room to center any photo point even when the stored preview is narrower than the viewport.
  image.style.margin = zoom ? `${height / 2}px ${width / 2}px` : '0';
  for (const target of [image, $('compare-image')]) {
    const sourceWidth = Number(target.getAttribute('width'));
    const sourceHeight = Number(target.getAttribute('height'));
    if (!sourceWidth || !sourceHeight) continue;
    const scale = target === image && zoom ? zoom : Math.max(0, Math.min(width / sourceWidth, height / sourceHeight));
    Object.assign(target.style, { width: `${sourceWidth * scale}px`, height: `${sourceHeight * scale}px`, maxWidth: 'none', maxHeight: 'none' });
  }
  if (!zoom) {
    $('navigator-viewport').hidden = true;
  }
  if (!review.comparing) updateZoomControls(zoom);
}
function updateZoomControls(value: number) {
  $('zoom-level').textContent = value ? `${Math.round(value * 100)}%` : '';
  $('navigator-fit').classList.toggle('active', value === 0);
  $('navigator-100').classList.toggle('active', value === 1);
  $('navigator-50').classList.toggle('active', value === .5);
}
function chooseZoom(value: number) {
  if (review.comparing) review.zoom(value);
  else { zoom = value; applyZoom(); }
}
function capturePhotoAnchor() {
  if (!zoom || viewMode !== 'loupe') return null;
  const image = $('full-image');
  const rect = image.getBoundingClientRect();
  // Rapid navigation can select another photo while the previous one is still loading.
  if (image.hidden || !rect.width || !rect.height) return pendingPhotoAnchor;
  const area = $('image-area');
  const viewport = area.getBoundingClientRect();
  const clientX = viewport.left + area.clientLeft + area.clientWidth / 2;
  const clientY = viewport.top + area.clientTop + area.clientHeight / 2;
  return { x: (clientX - rect.left) / rect.width, y: (clientY - rect.top) / rect.height, clientX, clientY };
}
function positionZoomAnchor(anchor: Anchor) {
  const rect = $('full-image').getBoundingClientRect();
  const area = $('image-area');
  area.scrollLeft += rect.left + anchor.x * rect.width - anchor.clientX;
  area.scrollTop += rect.top + anchor.y * rect.height - anchor.clientY;
  // Keep the focal point when an asynchronous high-quality preview replaces this image.
  zoomAnchor = anchor;
}
function pinchZoom(factor: number, clientX: number, clientY: number) {
  const image = $('full-image');
  if (viewMode !== 'loupe' || image.hidden || !Number.isFinite(factor) || factor <= 0) return;
  const before = image.getBoundingClientRect();
  if (!before.width || !before.height) return;
  const scale = before.width / Number(image.getAttribute('width'));
  const next = Math.min(4, Math.max(Math.min(.05, scale), scale * factor));
  if (next === scale) return;
  const x = (clientX - before.left) / before.width;
  const y = (clientY - before.top) / before.height;
  zoom = next;
  applyZoom();
  positionZoomAnchor({ x, y, clientX, clientY });
}
let pinchScale: number | null = null;
// WebKit emits gesture events; other webviews report trackpad pinches as Ctrl+wheel.
$('viewer').addEventListener('wheel', event => {
  if (!event.ctrlKey) zoomAnchor = null;
  if (viewMode !== 'loupe' || $('full-image').hidden) return;
  event.preventDefault();
  const area = $('image-area');
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? area.clientHeight : 1;
  if (!event.ctrlKey) {
    // Apply both axes directly to avoid WebKit's native scroll axis locking.
    area.scrollLeft += event.deltaX * (event.deltaMode === 2 ? area.clientWidth : unit);
    area.scrollTop += event.deltaY * unit;
    return;
  }
  if (pinchScale !== null) return;
  pinchZoom(Math.exp(-event.deltaY * unit * .01), event.clientX, event.clientY);
}, { passive: false });
$('viewer').addEventListener('pointerdown', () => { zoomAnchor = null; });
$('viewer').addEventListener('gesturestart', event => {
  if (viewMode !== 'loupe' || $('full-image').hidden) return;
  event.preventDefault();
  pinchScale = 1;
}, { passive: false });
$('viewer').addEventListener('gesturechange', event => {
  if (pinchScale === null) return;
  event.preventDefault();
  if (!Number.isFinite(event.scale) || event.scale <= 0) return;
  pinchZoom(event.scale / pinchScale, event.clientX, event.clientY);
  pinchScale = event.scale;
}, { passive: false });
document.addEventListener('gestureend', event => {
  if (pinchScale === null) return;
  event.preventDefault();
  pinchScale = null;
}, { passive: false });
async function navigate(delta: number, fromId: number | null = null) {
  let index = selected + delta;
  if (review.comparing) {
    const candidate = items.findIndex(item => item.id === (fromId ?? review.candidateId));
    index = (candidate >= 0 ? candidate : selected) + delta;
    if (items[index]?.id === review.referenceId) index += delta;
  }
  const token = epoch;
  if (index >= items.length && hasMore) await more();
  if (token !== epoch) return;
  if (await selectPhoto(index) && viewMode === 'grid') gridTiles.get(selected)?.scrollIntoView({ block: 'nearest' });
}
function toggleFilters(show = $('filter-panel').hidden) {
  $('filter-panel').hidden = !show;
  $('toggle-filters').setAttribute('aria-expanded', String(show));
  scheduleDraw();
}
function togglePanel(id: string) {
  $(id).hidden = !$(id).hidden;
  if(id==='library-inspector'&&!$(id).hidden&&items[selected])loadPhotoInfo(items[selected]);
  updatePanelButton(id);
  scheduleDraw();
}
function updatePanelButton(id: string) {
  const button = $(id === 'sidebar' ? 'toggle-sidebar' : 'toggle-inspector');
  button.setAttribute('aria-expanded', String(!$(id).hidden));
  button.textContent = $(id).hidden ? (id === 'sidebar' ? '▸' : '◂') : (id === 'sidebar' ? '◂' : '▸');
}
$('ai-selection-clear').onclick = () => { aiIds = []; $('ai-selection').hidden = true; refresh(); };
$('all').onclick = () => { aiIds = []; $('ai-selection').hidden = true; rootId = null; folderPath = null; $('title').textContent = t("すべての写真"); updateFolderSelection(); refresh(); };
for (const id of ['add', 'add-small', 'empty-add'] as const) $(id).onclick = addFolder;
$('folder-search').oninput = renderFolders;
$('scan').onclick = () => scan([rootId]);
$('cancel').onclick = () => {
  scanCancelled = true;
  invoke('cancel_scan').catch(showError);
};
$('search').oninput = () => { clearTimeout(searchTimer); searchTimer = setTimeout(refresh, 120); };
$('sort').onchange = () => refresh();
$('rating-filter').onchange = () => {libraryFilters.attribute();refresh();};
$('descending').onclick = () => {
  const descending = $('descending').getAttribute('aria-pressed') !== 'true';
  $('descending').setAttribute('aria-pressed', String(descending));
  $('descending').textContent = descending ? '↓' : '↑';
  refresh();
};
$('tile-size').oninput = () => { viewport.scrollTop = 0; scheduleDraw(); };
$('toggle-sidebar').onclick = () => togglePanel('sidebar');
$('toggle-inspector').onclick = () => togglePanel(develop.active ? 'develop-panel' : 'library-inspector');
$('toggle-modulebar').onclick = () => {
  $('module-bar').hidden = !$('module-bar').hidden;
  $('toggle-modulebar').setAttribute('aria-expanded', String(!$('module-bar').hidden));
  $('toggle-modulebar').textContent = $('module-bar').hidden ? '▾' : '▴';
  scheduleDraw();
};
$('toggle-filters').onclick = () => toggleFilters();
$('toggle-filmstrip').onclick = () => {
  filmstrip.hidden = !filmstrip.hidden;
  $('toggle-filmstrip').setAttribute('aria-expanded', String(!filmstrip.hidden));
  $('toggle-filmstrip').textContent = filmstrip.hidden ? '▴' : '▾';
  scheduleDraw();
};
$('annotation-form').oninput = () => { if (metadataReady) metadataDirty = true; };
$('annotation-form').onsubmit = saveAnnotation;
$('rating-stars').onclick = event => {
  const button = (event.target as Element).closest<HTMLButtonElement>('[data-rating]');
  if (!button || !metadataReady || saving) return;
  const rating = Number(button.dataset.rating);
  $('annotation-rating').value = String(items[selected].rating === rating ? 0 : rating);
  metadataDirty = true;
  saveAnnotation();
};
$('selection-reveal').onclick = () => { if (items[selected]) invoke('reveal', { id: items[selected].id }).catch(showError); };
$('grid-view').onclick = () => setViewMode('grid');
$('film-grid').onclick = () => setViewMode('grid');
$('loupe-view').onclick = () => setViewMode('loupe');
$('compare-view').onclick = () => setViewMode('compare');
$('survey-view').onclick = () => setViewMode('survey');
$('film-previous').onclick = () => navigate(-1);
$('film-next').onclick = () => navigate(1);
$('high-quality').onclick = () => develop.active ? highQuality() : setModule('develop');
$('zoom-in').onclick = () => { if (review.comparing) review.zoomBy(1.5); else chooseZoom(Math.min(4, zoom ? zoom * 1.5 : 1)); highQuality(); };
$('zoom-out').onclick = () => { if (review.comparing) review.zoomBy(1 / 1.5); else chooseZoom(zoom > .3 ? zoom / 1.5 : 0); };
$('navigator-fit').onclick = () => chooseZoom(0);
for (const button of $('photo-flags').querySelectorAll('button')) button.onclick = () => {
  const flag = Number(button.dataset.flag);
  setPhotoAttributes({flag:items[selected]?.flag === flag ? 0 : flag});
};
$('photo-rotate-left').onclick = () => rotatePhoto(-1);
$('photo-rotate-right').onclick = () => rotatePhoto(1);
$('photo-label').onchange=()=>setPhotoAttributes({label:$('photo-label').value});
$('navigator-100').onclick = () => { chooseZoom(1); highQuality(); };
$('navigator-50').onclick = () => { chooseZoom(.5); highQuality(); };
$('full-image').onclick = event => {
  const image = $('full-image').getBoundingClientRect();
  const area = $('image-area');
  const viewport = area.getBoundingClientRect();
  zoom = zoom ? 0 : 1;
  applyZoom();
  if (!zoom || !image.width || !image.height) return;
  positionZoomAnchor({
    x: (event.clientX - image.left) / image.width,
    y: (event.clientY - image.top) / image.height,
    clientX: viewport.left + area.clientLeft + area.clientWidth / 2,
    clientY: viewport.top + area.clientTop + area.clientHeight / 2,
  });
  highQuality();
};
initSidebar(() => { scheduleDraw(); updateNavigatorViewport(); });
viewport.onscroll = filmstrip.onscroll = scheduleDraw;
new ResizeObserver(scheduleDraw).observe($('workspace'));
new ResizeObserver(applyZoom).observe($('image-area'));
const navigatorObserver = new ResizeObserver(updateNavigatorViewport);
for (const id of ['full-image', 'navigator-image', 'image-area']) navigatorObserver.observe($(id));
$('image-area').addEventListener('scroll', updateNavigatorViewport, { passive: true });
$('navigator-image').addEventListener('load', updateNavigatorViewport);

async function pastePhotoSettings() {
  const id = items[selected]?.id;
  if (id == null || !await allowChange() || items[selected]?.id !== id) return;
  if (await develop.pasteSettings(id) && !develop.active && items[selected]?.id === id) {
    syncReview();
    await displayPhoto();
  }
}

document.addEventListener('keydown', event => {
  if ($('app-shell').inert || document.querySelector('dialog[open]') || photoExport.busy || !$('photo-menu').hidden || document.body.dataset.catalogBusy) return;
  if ((event.metaKey || event.ctrlKey) && event.key === ',') {
    event.preventDefault(); document.dispatchEvent(new Event('open-preferences')); return;
  }
  if ((event.target as Element).closest('[role=separator]')) return;
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f') {
    event.preventDefault(); toggleFilters(true); libraryFilters.text(); $('search').focus(); return;
  }
  if (!(event.target as Element).closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"]')
      && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey
      && ['c', 'v'].includes(event.key.toLowerCase()) && items[selected]) {
    event.preventDefault();
    if (!event.repeat && !develop.busy && !$('workspace').inert) {
      if (event.key.toLowerCase() === 'c') void develop.copySettings(items[selected].id);
      else void pastePhotoSettings().catch(showError);
    }
    return;
  }
  if ((event.target as Element).matches('input, textarea, select') || event.metaKey || event.ctrlKey || event.altKey) return;
  const key = event.key.toLowerCase();
  if (key === 'd') { event.preventDefault(); setModule('develop'); }
  else if (key === 'c' || key === 'n') { event.preventDefault(); setViewMode(key === 'c' ? 'compare' : 'survey'); }
  else if (key === 'g' || key === 'e') { event.preventDefault(); setViewMode(key === 'g' ? 'grid' : 'loupe'); }
  else if (key === ' ' && items[selected]) { event.preventDefault(); setViewMode(viewMode === 'grid' ? 'loupe' : 'grid'); }
  else if (key === 'escape') { event.preventDefault(); setViewMode('grid'); }
  else if (key === 'arrowright' || key === 'arrowleft') { event.preventDefault(); navigate(key === 'arrowright' ? 1 : -1); }
  else if (key === '\\') { event.preventDefault(); toggleFilters(); }
  else if (key === 'f5' || key === 'f6') { event.preventDefault(); $(key === 'f5' ? 'toggle-modulebar' : 'toggle-filmstrip').click(); }
  else if (['p','x','u'].includes(key)) {event.preventDefault();setPhotoAttributes({flag:key==='p'?1:key==='x'?-1:0});}
  else if (/^[0-5]$/.test(key) && metadataReady && !saving) {
    $('annotation-rating').value = key;
    metadataDirty = true;
    saveAnnotation();
  }
});
await listen<PreviewProgress>('preview-progress', event => {
  const report = event.payload;
  $('progress').hidden = false;
  $('progress-text').textContent = t("プレビュー {0}/{1}枚を準備・{2}枚失敗", report.ready.toLocaleString(), report.total.toLocaleString(), report.failed.toLocaleString());
  if (scanViewPending && report.ready > 0) {
    scanViewPending = false;
    loadFolders().then(() => refresh(report.last_ready_id)).catch(showError);
  }
});
await listen('library-preview-failed', event => showError(event.payload));
await listen<number>('library-preview-updated', event => {
  const id = event.payload;
  invalidatePhotoPreview(id); syncReview();
  if (!develop.active && items[selected]?.id === id) displayPhoto();
});
async function prepareExistingPreviews() {
  scanning = true;
  $('scan').disabled = true;
  for (const id of ['add', 'add-small', 'empty-add'] as const) $(id).disabled = true;
  try {
    const avifSupported = await new Promise<boolean>(resolve => {
      const image = new Image();
      image.onload = () => resolve(image.naturalWidth === 2);
      image.onerror = () => resolve(false);
      image.src = 'data:image/avif;base64,AAAAIGZ0eXBhdmlmAAAAAGF2aWZtaWYxbWlhZk1BMUEAAADrbWV0YQAAAAAAAAAhaGRscgAAAAAAAAAAcGljdAAAAAAAAAAAAAAAAAAAAAAOcGl0bQAAAAAAAQAAAB5pbG9jAAAAAEQAAAEAAQAAAAEAAAETAAAAFgAAAChpaW5mAAAAAAABAAAAGmluZmUCAAAAAAEAAGF2MDFDb2xvcgAAAABqaXBycAAAAEtpcGNvAAAAFGlzcGUAAAAAAAAAAgAAAAIAAAAQcGl4aQAAAAADCAgIAAAADGF2MUOBIAAAAAAAE2NvbHJuY2x4AAEADQAGgAAAABdpcG1hAAAAAAAAAAEAAQQBAoMEAAAAHm1kYXQSAAoEOAA2CTIMFwAIIIIEAAVAvMFA';
    });
    const report = await invoke('prepare_library_previews',{avifSupported});
    if (report?.error) showError(report.error);
  } catch (failure) { showError(failure); }
  finally {
    scanning = false; $('scan').disabled = false;
    for (const id of ['add', 'add-small', 'empty-add'] as const) $(id).disabled = false;
    $('progress').hidden = true;
  }
}
document.querySelector('.search kbd')!.textContent = /Mac/.test(navigator.platform) ? '⌘ F' : 'Ctrl F';
let catalogReady = false;
let needsCatalog = false;
try {
  await listen('close-requested', async () => { if (needsCatalog || (!$('app-shell').inert && await allowChange())) invoke('close_window').catch(showError); });
  await invoke('arm_close_guard');
  await listen<ScanProgress>('scan-progress', event => {
    $('progress-text').textContent = t("{0}件を確認・{1}枚を登録", event.payload.visited.toLocaleString(), event.payload.indexed.toLocaleString());
  });
  needsCatalog = await invoke('catalog_needs_creation');
  if (needsCatalog) {
    $('startup-screen').removeAttribute('role');
    $('startup-screen').setAttribute('aria-label', t('カタログを作成'));
    $('startup-loading').hidden = true;
    $('catalog-setup').hidden = false;
  } else {
    const catalogPath = await invoke('catalog_path');
    $('catalog-location').title = catalogPath;
    $('catalog-name').textContent = catalogPath.split(/[\\/]/).at(-1)!.replace(/\.(openroom|db)$/i, '');
    $('catalog-name').title = catalogPath;
    await loadFolders(); await refresh(null, true);
    catalogReady = true;
    prepareExistingPreviews();
  }
}
catch (error) { showError(error); $('status').textContent = t("カタログを開けませんでした"); }
finally {
  $('startup-screen').hidden = !needsCatalog;
  $('app-shell').inert = needsCatalog;
}

function mcpUiState(): UiState {
  return {
    selected_ids: [...selectedPhotos], active_id: items[selected]?.id || null,
    view: develop.active ? 'develop' : viewMode, query: query(),
    unsaved: { annotations: metadataDirty, develop: develop.dirty },
  };
}
async function prepareMcpChange() {
  if (metadataDirty) throw Error(t("未保存のタグ・評価・メモがあります。画面で保存してください。"));
  if (photoExport.busy || saving || confirmingChange || scanning || document.body.dataset.catalogBusy) throw Error(t("画面が処理中です。完了後に再試行してください。"));
  await developSelectionTask;
  if (!await develop.allowChange()) throw Error(t("現像設定を保存できません。画面の編集内容を保持しています。"));
}
async function refreshMcpPhotos(ids: number[]) {
  const keepId = items[selected]?.id, selectedIds = [...selectedPhotos];
  const changed = ids.length ? ids : items.map(item => item.id);
  for (const id of changed) developVersions.set(id, (developVersions.get(id) || 0) + 1);
  thumbnailRequests.clear(); readyThumbnails.clear(); largePreviews.clear(); photoMetadata.clear();
  await loadFolders();
  await refresh(keepId, true);
  for (const id of selectedIds) if (items.some(item => item.id === id)) selectedPhotos.add(id);
  $('selection-count').textContent = t("/ {0}枚選択 /", selectedPhotos.size);
  syncReview(); draw();
  await developSelectionTask;
}
try {
  if (!needsCatalog) await startMcp({ invoke, listen, ready: catalogReady, state: mcpUiState, prepare: prepareMcpChange,
    busy: () => photoExport.busy || metadataDirty || saving || confirmingChange || scanning || develop.busy || Boolean(document.body.dataset.catalogBusy),
    async select(ids) {
      aiIds = [...ids]; rootId = null; folderPath = null;
      libraryFilters.clear(); $('search').value = ''; $('rating-filter').value = '';
      $('ai-selection').hidden = false; $('title').textContent = t("AI選択：{0}枚", ids.length);
      updateFolderSelection(); await refresh(ids[0], true);
      if (items[selected]?.id !== ids[0]) throw Error(t("指定した写真を選択できませんでした"));
      selectedPhotos.clear(); for (const id of ids) selectedPhotos.add(id);
      $('selection-count').textContent = t("/ {0}枚選択 /", ids.length); syncReview(); draw();
      await developSelectionTask;
    },
    async view(name) {
      if (!['grid', 'loupe', 'compare', 'survey', 'develop'].includes(name)) throw Error(t("表示モードが不正です"));
      if (name === 'develop') await setModule('develop');
      else { await setModule('library'); await setViewMode(name); }
      await developSelectionTask;
    },
    refresh: refreshMcpPhotos, error: showError,
    async changeLanguage(value) {
      if (scanning || saving || rotating || confirmingChange || photoExport.busy || document.body.dataset.catalogBusy) throw Error(t("処理が終わってから言語を変更してください。"));
      await developSelectionTask;
      if (metadataDirty) await saveAnnotation();
      if (metadataDirty) throw Error(t("保存できませんでした。編集内容を保持しています。"));
      if (!await develop.allowChange()) throw Error(t("現像設定を保存できません。画面の編集内容を保持しています。"));
      await invoke('set_language', {language:value});
      location.reload();
    },
  });
} catch (failure) { showError(t("AI連携を初期化できません：{0}", failure)); }
