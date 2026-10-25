import { createView } from './components/view';
import type { Photo, Preview, Focus } from './models';
interface Card { photo: Photo; element: HTMLElement; heading: HTMLButtonElement; area: HTMLDivElement; image: HTMLImageElement; message: HTMLParagraphElement; attributes: HTMLSpanElement; exclude: HTMLButtonElement; zoom: number; focus: Focus; request: number; bound: number; key: string; syncedScroll?: [number, number] }
import { element as $ } from './dom';
import { t } from './i18n';
// Library comparison selects photos; development comparison selects recipe states.

export function createReview({ preview, src, activate, remove, menu, next, zoomed, key }: { preview(photo: Photo, bound: number, retry: boolean): Promise<Preview>; src(path: string): string; activate(id: number): Promise<boolean>; remove(id: number): Promise<void>; menu(id: number, event: MouseEvent): void; next(id: number): Promise<void>; zoomed(value: number): void; key(photo: Photo): string }) {
  const stage = $('review-photos');
  const view = createView(stage);
  const cards = new Map<number | null | undefined, Card>();
  let mode = '', photos: Photo[] = [], activeId: number | null | undefined = null, referenceId: number | null = null, candidateId: number | null = null;
  let generation = 0, queued: Card[] = [], workers = 0;
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) if (entry.isIntersecting) {
      observer.unobserve(entry.target);
      const card = cards.get(Number((entry.target as HTMLElement).dataset.id));
      if (card) { queued.push(card); pump(); }
    }
  }, { root: stage, rootMargin: '150px' });
  function pump() {
    while (workers < 2 && queued.length) {
      const card = queued.shift()!;
      if (cards.get(card.photo.id) !== card) continue;
      workers++;
      load(card, card.bound || 1600).finally(() => { workers--; pump(); });
    }
  }
  function center(card: Card) {
    const area = card.area, image = card.image;
    if (!image.hasAttribute('width')) return card.focus;
    return {
      x: image.offsetWidth ? (area.scrollLeft + area.clientWidth / 2 - image.offsetLeft) / image.offsetWidth : .5,
      y: image.offsetHeight ? (area.scrollTop + area.clientHeight / 2 - image.offsetTop) / image.offsetHeight : .5,
    };
  }
  function position(card: Card, point: Focus) {
    card.area.scrollLeft = card.image.offsetLeft + point.x * card.image.offsetWidth - card.area.clientWidth / 2;
    card.area.scrollTop = card.image.offsetTop + point.y * card.image.offsetHeight - card.area.clientHeight / 2;
    card.syncedScroll = [card.area.scrollLeft, card.area.scrollTop];
    card.focus = point;
  }
  function size(card: Card, point = { x: .5, y: .5 }) {
    const width = Number(card.image.getAttribute('width')), height = Number(card.image.getAttribute('height'));
    if (!width || !height) return;
    const fit = Math.min(card.area.clientWidth / width, card.area.clientHeight / height);
    const scale = card.zoom || fit;
    card.image.style.width = `${width * scale}px`;
    card.image.style.height = `${height * scale}px`;
    position(card, point);
  }
  async function load(card: Card, bound: number, retry = false) {
    card.bound = bound;
    const token = ++card.request;
    const current = generation;
    card.message.textContent = t("読み込み中…"); card.message.hidden = false;
    try {
      const result = await preview(card.photo, bound, retry);
      if (current !== generation || cards.get(card.photo.id) !== card || token !== card.request) return;
      const point = center(card);
      card.image.width = result.render_width;
      card.image.height = result.render_height;
      card.image.onload = () => {
        if (cards.get(card.photo.id) !== card || token !== card.request) return;
        card.message.hidden = true; size(card, card.focus);
      };
      card.image.onerror = () => { card.message.textContent = t("画像を読み込めません。「再読み込み」で再試行してください。"); card.message.hidden = false; };
      card.image.src = result.inline_image ?? src(result.path);
      card.bound = bound;
      size(card, point);
    } catch (failure) {
      if (current === generation && token === card.request) { card.message.textContent = String(failure); card.message.hidden = false; }
    }
  }
  function targets(card: Card) { return $('compare-linked').checked ? [...cards.values()] : [card]; }
  function zoom(value: number, point?: Focus) {
    const card = cards.get(activeId) || cards.get(candidateId) || cards.values().next().value;
    if (!card || mode !== 'compare') return;
    if (!Number.isFinite(value) || value < 0 || value > 4) return;
    const focus = point || center(card);
    for (const target of targets(card)) { target.zoom = value; size(target, focus); }
    zoomed(value);
  }
  function zoomBy(factor: number) {
    const card = cards.get(activeId) || cards.get(candidateId);
    if (!card || !Number.isFinite(factor) || factor <= 0) return;
    const scale = card.zoom || card.image.offsetWidth / Number(card.image.getAttribute('width'));
    if (Number.isFinite(scale) && scale > 0) zoom(Math.max(.05, Math.min(4, scale * factor)));
  }
  async function choose(id: number) {
    if (!await activate(id)) return false;
    activeId = id;
    update(photos, id);
    return true;
  }
  function makeCard(photo: Photo, element: HTMLElement) {
    const heading = element.querySelector<HTMLButtonElement>('.review-heading')!;
    const area = element.querySelector<HTMLDivElement>('.review-image-area')!;
    const image = area.querySelector('img')!;
    const message = area.querySelector('p')!;
    const attributes = element.querySelector<HTMLSpanElement>('.review-attributes')!;
    const exclude = element.querySelector<HTMLButtonElement>('.review-exclude')!;
    exclude.onclick = () => remove(photo.id);
    element.querySelector<HTMLButtonElement>('.review-retry')!.onclick = () => load(card, card.bound || 1600, true);
    const linked = mode === 'compare' && $('compare-linked').checked ? cards.get(referenceId) || cards.values().next().value : null;
    const card: Card = { photo, element, heading, area, image, message, attributes, exclude,
      zoom: linked?.zoom || 0, focus: linked ? center(linked) : {x:.5,y:.5}, request: 0, bound: linked?.bound || 0, key: key(photo) };
    heading.onclick = () => choose(photo.id);
    area.onfocus = () => { if (activeId !== photo.id) choose(photo.id); };
    area.onclick = () => choose(photo.id);
    area.ondblclick = async () => { if (await choose(photo.id)) zoom(card.zoom ? 0 : 1); };
    element.oncontextmenu = event => { event.preventDefault(); menu(photo.id, event); };
    let pan: {x:number;y:number;left:number;top:number} | null = null, gestureScale: number | null = null;
    area.onpointerdown = async event => {
      if (event.button !== 0 || mode !== 'compare') return;
      if (!await choose(photo.id)) return;
      if (!card.zoom || !event.buttons) return;
      area.setPointerCapture(event.pointerId);
      pan = { x: event.clientX, y: event.clientY, left: area.scrollLeft, top: area.scrollTop };
    };
    area.onpointermove = event => {
      if (!pan) return;
      area.scrollLeft = pan.left + pan.x - event.clientX;
      area.scrollTop = pan.top + pan.y - event.clientY;
      syncPan(card);
    };
    area.onpointerup = area.onpointercancel = () => { pan = null; };
    area.onscroll = () => {
      const [left, top] = card.syncedScroll || [];
      if (mode === 'compare' && (left !== area.scrollLeft || top !== area.scrollTop)) syncPan(card);
    };
    area.addEventListener('wheel', event => {
      if (mode !== 'compare') return;
      event.preventDefault();
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? area.clientHeight : 1;
      if (event.ctrlKey) {
        if (gestureScale !== null) return;
        choose(photo.id).then(ok => { if (ok) zoomBy(Math.exp(-event.deltaY * unit * .01)); });
      } else {
        area.scrollLeft += event.deltaX * unit; area.scrollTop += event.deltaY * unit;
        syncPan(card);
      }
    }, { passive: false });
    area.addEventListener('gesturestart', event => { if (mode === 'compare') { event.preventDefault(); gestureScale = 1; } }, { passive: false });
    area.addEventListener('gesturechange', event => {
      if (gestureScale === null || !Number.isFinite(event.scale) || event.scale <= 0) return;
      event.preventDefault(); const factor = event.scale / gestureScale; gestureScale = event.scale;
      choose(photo.id).then(ok => { if (ok) zoomBy(factor); });
    }, { passive: false });
    area.addEventListener('gestureend', () => { gestureScale = null; });
    return card;
  }
  function syncPan(card: Card) {
    const point = center(card);
    card.focus = point;
    if (!$('compare-linked').checked || !card.zoom) return;
    for (const other of cards.values()) if (other !== card) position(other, point);
  }
  function layout() {
    if (!mode) return;
    const count = mode === 'compare' ? 2 : Math.max(1, cards.size);
    const ratio = stage.clientWidth / Math.max(1, stage.clientHeight);
    const columns = mode === 'compare' ? 2 : Math.min(count, Math.max(1, Math.min(Math.floor(stage.clientWidth / 180), Math.round(Math.sqrt(count * ratio)))));
    stage.style.gridTemplateColumns = `repeat(${columns}, minmax(0, 1fr))`;
    const rows = Math.ceil(count / columns);
    stage.style.gridAutoRows = `${Math.max(150, (stage.clientHeight - 20 - 10 * (rows - 1)) / rows)}px`;
    for (const card of cards.values()) size(card, center(card));
  }
  new ResizeObserver(layout).observe(stage);
  function update(selection: Photo[], focused: number | null | undefined) {
    photos = selection; activeId = focused;
    if (!mode) return;
    const ids = photos.map(photo => photo.id);
    if (ids.length && !(referenceId !== null && ids.includes(referenceId))) referenceId = ids[0] ?? null;
    if (ids.length && (!(candidateId !== null && ids.includes(candidateId)) || candidateId === referenceId)) candidateId = ids.find(id => id !== referenceId) ?? null;
    if (mode === 'compare' && focused != null && ids.includes(focused) && focused !== referenceId && !cards.has(focused)) candidateId = focused;
    const visible = mode === 'compare' ? photos.filter(photo => photo.id === referenceId || photo.id === candidateId).sort((a,b) => Number(b.id === referenceId) - Number(a.id === referenceId)) : photos;
    const visibleIds = new Set(visible.map(photo => photo.id));
    for (const [id, card] of cards) if (id == null || !visibleIds.has(id)) { observer.unobserve(card.element); cards.delete(id); }
    view.render(visible.map(photo => <article key={photo.id} className={`review-card${photo.id === activeId ? ' active' : ''}`} tabIndex={-1} data-id={photo.id}
      style={{order: mode === 'compare' && photo.id === referenceId ? -1 : 0}} ref={element => {
        if (element && !cards.has(photo.id)) {
          const card = makeCard(photo, element); cards.set(photo.id, card); observer.observe(element);
        }
      }}>
      <button className="review-heading" aria-pressed={photo.id === activeId}>{`${mode === 'compare' ? photo.id === referenceId ? t('基準：') : t('候補：') : ''}${photo.name}`}</button>
      <div className="review-image-area" tabIndex={0} aria-label={photo.name}><img alt={photo.name} draggable={false} decoding="async" /><p className="review-load" role="status" /></div>
      <div className="review-footer"><span className="review-attributes">{`${'★'.repeat(photo.rating) || t('未評価')}${photo.flag === 1 ? t(' · 採用') : photo.flag === -1 ? t(' · 除外') : ''}${photo.label ? ` · ${photo.label}` : ''}`}</span>
        <button className="review-retry">{t('再読み込み')}</button><button className="review-exclude" disabled={photos.length <= 1} title={t('選択と表示から外します。原本は削除しません。')}>{t('候補から外す')}</button>
      </div>
    </article>));
    for (const photo of visible) {
      const card = cards.get(photo.id)!;
      card.photo = photo;
      if (card.key !== key(photo)) { card.key = key(photo); queued.push(card); pump(); }
    }
    $('review-message').textContent = mode === 'compare' ? candidateId ? t("基準と候補を比較。写真をクリックして評価対象を選択。← →で候補を移動。") : t("比較相手をフィルムストリップから選択してください。") : t("{0}枚の候補 · 写真をクリックして評価。「候補から外す」は原本を削除しません。", photos.length);
    $('compare-swap').disabled = $('compare-promote').disabled = !candidateId;
    if (mode === 'compare') zoomed((cards.get(activeId) || cards.get(candidateId))?.zoom || 0);
    layout();
  }
  $('compare-swap').onclick = () => { [referenceId, candidateId] = [candidateId, referenceId]; update(photos, activeId); };
  $('compare-promote').onclick = async () => {
    if (!candidateId || !await choose(candidateId)) return;
    referenceId = candidateId; candidateId = null; update(photos, activeId); await next(referenceId);
  };
  $('compare-linked').onchange = () => { if ($('compare-linked').checked) zoom((cards.get(activeId) || cards.get(candidateId))?.zoom || 0); };
  return {
    get active() { return Boolean(mode); },
    get comparing() { return mode === 'compare'; },
    get referenceId() { return referenceId; },
    get candidateId() { return candidateId; },
    show(value: string, selection: Photo[], focused: number | null | undefined) {
      if (value !== mode) { generation++; observer.disconnect(); queued = []; cards.clear(); view.render(null); referenceId = focused ?? null; candidateId = null; }
      mode = value; $('review-view').hidden = !mode; $('compare-controls').hidden = mode !== 'compare';
      update(selection, focused);
    },
    update, zoom, zoomBy,
  };
}
