import { flushSync } from 'react-dom';
import { createView } from './components/view';
import { AdjustmentSlider, type SliderHandle } from './components/AdjustmentSlider';
import type { ReactNode } from 'react';
import type { Invoke } from './backend';
import type { Settings, SettingsPatch, SettingKey, NumericSetting, Photo, PhotoMetadata, DevelopedPreview, RawInfo, HistoryRow, ErrorHandler } from './models';
type SliderPath = [NumericSetting] | ['local', string] | ['curves' | 'mixer' | 'grading', number, number] | ['crop', number];
import { element as $ } from './dom';
import { t, language, metadataValue } from './i18n';
import { createToneCurve } from "./tone-curve";
import { createDevelopTools } from "./develop-tools";
import { createCopySettings } from './components/CopySettings';
// Rust owns image processing; this controller only edits recipes and displays results.

const clone = <T,>(value: T): T => structuredClone(value);
const equal = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const fields: Record<string, [NumericSetting, string, number, number, number?][]> = {
  basic: [
    ['raw_temperature', t("色温度（K）"), 2000, 60000], ['raw_tint', t("色かぶり補正"), -150, 150],
    ['exposure', t("露光量"), -5, 5, .05], ['contrast', t("コントラスト"), -100, 100],
    ['highlights', t("ハイライト"), -100, 100], ['shadows', t("シャドウ"), -100, 100],
    ['whites', t("白レベル"), -100, 100], ['blacks', t("黒レベル"), -100, 100],
    ['texture', t("テクスチャ"), -100, 100], ['clarity', t("明瞭度"), -100, 100],
    ['dehaze', t("かすみの除去"), -100, 100], ['vibrance', t("自然な彩度"), -100, 100], ['saturation', t("彩度"), -100, 300],
  ],
  detail: [
    ['sharpen', t("シャープ量"), 0, 150], ['sharpen_radius', t("半径"), .5, 3, .1], ['sharpen_detail', t("ディテール"), 0, 100],
    ['sharpen_masking', t("エッジ保護"), 0, 100], ['noise_luma', t("輝度ノイズ低減"), 0, 100], ['noise_color', t("カラーノイズ低減"), 0, 100],
  ],
  geometry: [
    ['angle', t("角度"), -45, 45, .01], ['perspective_v', t("垂直方向"), -100, 100], ['perspective_h', t("水平方向"), -100, 100],
    ['scale', t("拡大・縮小"), 50, 200], ['aspect',t("縦横比の変形"),-100,100], ['offset_x',t("水平移動"),-100,100], ['offset_y',t("垂直移動"),-100,100], ['distortion', t("手動の歪曲補正"), -100, 100],
  ],
  effects: [
    ['vignette', t("周辺光量"), -100, 100], ['vignette_midpoint', t("中心点"), 0, 100], ['vignette_feather', t("ぼかし"), 0, 100],
    ['grain', t("粒子の適用量"), 0, 100], ['grain_size', t("粒子サイズ"), 0, 100], ['grain_roughness', t("粒子の粗さ"), 0, 100],
  ],
};
export function createDevelop({ invoke, show, compare, saved, error, selectedIds, photoInfo }: { invoke: Invoke; show(result: DevelopedPreview): void; compare(result: DevelopedPreview | null, mode?: string): void; saved(id: number, refreshPreview?: boolean): void; error: ErrorHandler; selectedIds(): number[]; photoInfo(item: Photo): Promise<PhotoMetadata> }) {
  let active = false, item: Photo | null = null, settings: Settings | null = null, defaults: Settings | null = null, baseline: Settings | null = null, revision = 0;
  let loading = false, saving = false, exporting = false, before = false, bound = 1600;
  let epoch = 0, renderToken = 0, running: false | { bound: number } = false, pending = false, timer: ReturnType<typeof setTimeout> | undefined;
  let gestureTimer: ReturnType<typeof setTimeout> | undefined, saveTimer: ReturnType<typeof setTimeout> | undefined, saveTask: Promise<boolean> | null = null, saveFailed = false, syncFailed = false, leaving = false;
  let undo: Settings[] = [], redo: Settings[] = [], clipboard: SettingsPatch | null = null, gesture: Settings | null = null, gestureOwner: HTMLElement | null = null;
  let interacting = false;
  let pendingStep: {label: string} | null = null, historyRows: HistoryRow[] = [], historyToken = 0;
  const chooseCopySettings = createCopySettings();
  const controls: SliderHandle[] = [];
  const historyView = createView($('develop-history-list'));
  let rawInfo: RawInfo | null = null, rasterTemperature: number | null = null, whiteBalanceReady = false;
  const tools = createDevelopTools({ invoke, error, edit, refresh:schedule, save, status, saved, selectedIds,
    selectionChanged() { finishGesture(); sync(); },
    get:()=>({settings,defaults}),
    rendering: () => Boolean(running || pending || loading),
    finishGesture() { if (gestureOwner === null) finishGesture(); },
    interact(change) {
      if (!settings || loading || leaving || exporting) return;
      beginGesture(); change(settings); changed();
    },
    busy(value){if(value!==undefined){exporting=value;buttons();}return exporting||leaving;},
  });
  function prepareGesture(owner: HTMLElement | null) {
    if (gesture && gestureOwner !== owner) finishGesture();
    gestureOwner = owner;
    if (!gesture && settings) gesture = clone(settings);
  }
  function beginGesture(owner: HTMLElement | null = null) {
    if (!settings || loading || leaving || exporting) return;
    prepareGesture(owner);
    if (interacting) return;
    interacting = true;
    clearTimeout(saveTimer);
    if (running && running.bound !== bound) {
      ++renderToken;
      invoke('cancel_develop_preview').catch(error);
      running = false;
    }
  }
  const toneCurve = createToneCurve({get:()=>settings,begin:beginGesture,
    change(fn){if(settings&&!leaving&&!exporting){fn(settings);changed();}},end:finishGesture,edit,blocked:()=>!settings||loading||leaving||exporting});
  function wbField(key: 'raw_temperature'): 'raw_temperature' | 'raster_temperature';
  function wbField(key: 'raw_tint'): 'raw_tint' | 'raster_tint';
  function wbField(key: 'raw_temperature' | 'raw_tint') {
    return rawInfo ? key : key === 'raw_temperature' ? 'raster_temperature' : 'raster_tint';
  }
  function numericValue(recipe: Settings, path: Exclude<SliderPath, ['local', string]>): number {
    if (path.length === 3) return recipe[path[0]][path[1]][path[2]];
    if (path.length === 2) return recipe.crop[path[1]];
    return recipe[path[0]] ?? 0;
  }
  function get(path: SliderPath): number {
    if (!settings) return 0;
    if (path[0] === 'local') return tools.selectedMask()?.adjustments[path[1]] ?? 0;
    if (path[0] === 'raw_temperature') return settings[wbField('raw_temperature')]
      ?? Math.max(2000, Math.min(60000, (rawInfo?.as_shot?.[0] ?? rasterTemperature ?? 6504) * 2 ** (settings.temperature / 200)));
    if (path[0] === 'raw_tint') return settings[wbField('raw_temperature')] == null
      ? Math.max(-150, Math.min(150, (rawInfo?.as_shot?.[1] ?? 0) + settings.tint)) : settings[wbField('raw_tint')];
    return numericValue(settings, path);
  }
  function put(path: SliderPath, value: number) {
    if (!settings) return;
    if (path[0] === 'local') {
      const mask = tools.selectedMask();
      if (mask) mask.adjustments[path[1]] = value;
      return;
    }
    if (path[0] === 'raw_temperature' || path[0] === 'raw_tint') {
      if (!rawInfo && settings.raster_reference_temperature == null) {
        settings.raster_reference_temperature = settings.raster_temperature == null ? rasterTemperature ?? 6504 : 6504;
      }
      if (settings[wbField('raw_temperature')] == null) {
        const temperature = get(['raw_temperature']), tint = get(['raw_tint']);
        settings[wbField('raw_temperature')] = Math.round(temperature);
        settings[wbField('raw_tint')] = Math.round(tint);
      }
      settings.temperature = 0; settings.tint = 0;
      if (path[0] === 'raw_temperature') settings[wbField('raw_temperature')] = value;
      else settings[wbField('raw_tint')] = value;
    } else if (path.length === 3) settings[path[0]][path[1]][path[2]] = value;
    else if (path.length === 2) settings.crop[path[1]] = value;
    else settings[path[0]] = value;
    if (path[0] === 'crop') {
      settings.crop[0] = Math.min(settings.crop[0], 1 - settings.crop[2]);
      settings.crop[1] = Math.min(settings.crop[1], 1 - settings.crop[3]);
    }
  }
  const dirty = () => Boolean(settings && (!equal(settings, baseline) || pendingStep));
  function status(text: string) { $('develop-status').textContent = text; }
  function buttons() {
    const ready = Boolean(item && settings && !loading && !leaving && !exporting);
    $('develop-controls').disabled = !ready;
    $<HTMLFieldSetElement>('mask-palette').disabled = !ready;
    $<HTMLFieldSetElement>('develop-left-controls').disabled = !ready;
    $('develop-save').disabled = !ready || (!dirty() && !syncFailed) || saving;
    $('develop-undo').disabled = !ready || !undo.length;
    $('develop-redo').disabled = !ready || !redo.length;
    $('develop-paste').disabled = !ready || !clipboard;
    $('develop-before').setAttribute('aria-pressed', String(before));
    $('develop-before').textContent = before ? t("変更前を表示中") : t("変更前と比較");
    $('develop-reset').disabled = !ready;
    $('develop-history-list').inert = !ready;
    $('develop-history-more').disabled = !ready;
  }
  function recordStep(previous: Settings | null, label?: string) {
    if (!previous || !settings || equal(previous, settings)) return;
    const changedKeys = (Object.keys(settings) as SettingKey[]).filter(key => !equal(previous[key], settings![key]));
    const labels = Object.values(fields).flat().filter(row => changedKeys.includes(row[0])).map(row => row[1]);
    pendingStep = { label: label || (changedKeys.length === 1 && labels.length ? labels[0] : t("設定変更")) };
  }
  function remember(previous: Settings, label?: string) {
    if (!equal(previous, settings)) { undo.push(previous); if (undo.length > 10) undo.shift(); redo = []; recordStep(previous, label); }
    buttons();
  }
  function finishGesture() {
    clearTimeout(gestureTimer);
    if (gesture && settings) remember(gesture);
    gesture = null; gestureOwner = null;
    if (interacting) {
      interacting = false;
      if (tools.comparison()) schedule();
      if (dirty()) saveTimer = setTimeout(save, 600);
    }
  }
  function sync() {
    if (!settings) { buttons(); return; }
    // MCP completion and native close guards must observe the committed controls.
    flushSync(() => { for (const control of controls) control.sync(); });
    $('develop-mono').checked = settings.monochrome;
    $('develop-flip-h').checked = settings.flip_h;
    $('develop-flip-v').checked = settings.flip_v;
    rawReset.hidden = !whiteBalanceReady;
    rawProfile.hidden = !rawInfo;
    rawStatus.hidden = !whiteBalanceReady;
    rawStatus.textContent = settings[wbField('raw_temperature')] != null ? t("カスタムWB")
      : settings.temperature || settings.tint ? t("旧設定のWB（K値は目安。入力するとK指定に切替）") : t("撮影時のホワイトバランス");
    if (!rawInfo) rawStatus.textContent += rasterTemperature != null
      ? t("。撮影時 {0}K（メタデータ取得）", Math.round(rasterTemperature).toLocaleString())
      : t("。推定基準 6,504K（撮影時のK値は不明）");
    drawCurve();
    tools.sync();
    buttons();
  }
  function changed() {
    before = false;
    saveFailed = false;
    status(interacting || dirty() ? t("編集中") : t("保存済み"));
    clearTimeout(saveTimer);
    if (dirty() && !interacting) saveTimer = setTimeout(save, 600);
    sync(); schedule();
  }
  function edit(change: (settings: Settings) => void, label?: string) {
    if (!settings || leaving || exporting) return;
    finishGesture();
    const previous = clone(settings);
    change(settings);
    remember(previous, label); changed();
  }
  function slider(path: SliderPath, label: string, min: number, max: number, step = 1, factor = 1) {
    const kelvin = path[0] === 'raw_temperature', key = path.join('-'), local = path[0] === 'local';
    const directions: Record<string, string> = {
      raw_temperature: t("左：青く／右：黄色く"), raw_tint: t("左：緑へ／右：マゼンタへ"),
      exposure: t("左：暗く／右：明るく"), contrast: t("左：柔らかく／右：強く"),
      highlights: t("左：明部を暗く／右：明部を明るく"), shadows: t("左：暗部を暗く／右：暗部を明るく"),
      whites: t("左：白を抑える／右：白を明るく"), blacks: t("左：黒を深く／右：黒を持ち上げる"),
      texture: t("左：模様を滑らかに／右：模様を強調"), clarity: t("左：柔らかく／右：局所的なコントラストを強調"),
      dehaze: t("左：かすみを加える／右：かすみを除去"),
      saturation: t("左：彩度を下げる／右：彩度を上げる"), vibrance: t("左：彩度を下げる／右：淡い色を鮮やかに"),
      sharpen_radius: t("シャープ量が0より大きいときに適用"), sharpen_detail: t("シャープ量が0より大きいときに適用"),
      sharpen_masking: t("シャープ量が0より大きいときに適用"),
      vignette_midpoint: t("周辺光量が0以外のときに適用"), vignette_feather: t("周辺光量が0以外のときに適用"),
      grain_size: t("粒子の適用量が0より大きいときに適用"), grain_roughness: t("粒子の適用量が0より大きいときに適用"),
      grading_blending: t("カラーグレーディングの明部・中間調・暗部の効果を混ぜる"),
      grading_balance: t("カラーグレーディングの明部・中間調・暗部の重みを変える"),
    };
    let title = directions[local ? path[1] : key] || '';
    if (path[0] === 'grading' && path[2] === 0) title = t('彩度が0より大きいときに色相を適用');
    function track() {
      if (path[0] !== 'mixer' && path[0] !== 'grading') return undefined;
      const mixerHue = [0, 30, 60, 120, 180, 240, 270, 300][path[1]];
      if (path[2] === 0) return path[0] === 'mixer'
        ? `linear-gradient(90deg,hsl(${mixerHue - 45} 60% 55%),hsl(${mixerHue} 60% 55%),hsl(${mixerHue + 45} 60% 55%))`
        : 'linear-gradient(90deg,#cb5858,#cecd58,#58c959,#58c8cb,#585acb,#c758cb,#cb5858)';
      const color = `hsl(${path[0] === 'mixer' ? mixerHue : settings?.grading[path[1]][0] ?? 0} 65% 55%)`;
      return path[2] === 1 ? `linear-gradient(90deg,#888,${color})` : `linear-gradient(90deg,#171717,${color},#eee)`;
    }
    return <AdjustmentSlider key={key} ref={handle => { if (handle) controls.push(handle); }} id={local ? key : `develop-${key}`} label={label} title={title}
      min={min} max={max} step={step} kelvin={kelvin} local={local} angle={key === 'angle'}
      state={() => ({ value: Number((get(path) * factor).toFixed(path[0].startsWith('raw_') ? 0 : 3)), disabled: local && !tools.selectedMask(), hidden: path[0].startsWith('raw_') && !whiteBalanceReady, track: track() })}
      prepare={prepareGesture} input={(value, input, numeric) => {
        if (!settings || leaving || exporting || (local && !tools.selectedMask())) return;
        beginGesture(input); put(path, value / factor);
        if (path[0] === 'curves') settings.point_curves[path[1]] = [];
        changed();
        if (numeric) { clearTimeout(gestureTimer); gestureTimer = setTimeout(finishGesture, 150); }
      }} finish={input => { if (gestureOwner === input) finishGesture(); }} sync={sync} levelGrid={tools.showLevelGrid}
      reset={() => {
        if (path[0].startsWith('raw_')) resetRawWb();
        else edit(() => { if (defaults) put(path, path[0] === 'local' ? 0 : numericValue(defaults, path)); });
      }} />;
  }
  function resetRawWb() {
    edit(settings => {
      settings[wbField('raw_temperature')] = null; settings[wbField('raw_tint')] = 0;
      if (!rawInfo) settings.raster_reference_temperature = null;
      settings.temperature = 0; settings.tint = 0;
    }, t("撮影時のホワイトバランス"));
  }
  for (const [group, rows] of Object.entries(fields)) {
    const content: ReactNode[] = [];
    const headings: Partial<Record<SettingKey, string>> = { raw_temperature: t('ホワイトバランス'), exposure: t('階調'), texture: t('外観') };
    for (const [key, label, min, max, step] of rows) {
      if (group === 'basic' && headings[key]) content.push(<h3 key={`heading-${key}`} className="adjustment-heading">{headings[key]}</h3>);
      if (key === 'raw_temperature') content.push(<p key="profile" id="raw-profile" className="develop-profile" hidden />, <p key="status" id="raw-status" className="develop-profile" hidden />, <button key="reset" id="raw-reset" type="button" className="develop-wb-reset" hidden onClick={resetRawWb}>{t('撮影時')}</button>);
      const control = slider([key], label, min, max, step);
      if (key === 'angle') createView($('straighten-adjustments')).render(control);
      else content.push(control);
    }
    createView($(`develop-${group}`)).render(content);
  }
  const rawProfile = $('raw-profile'), rawStatus = $('raw-status'), rawReset = $('raw-reset');
  createView($('local-adjustments')).render(fields.basic.map(([key, label, min, max, step]) => {
    const wb = key.startsWith('raw_');
    return slider(['local', key.replace('raw_', '')], key === 'raw_temperature' ? t('色温度（相対）') : label, wb ? -100 : min, wb ? 100 : max, step);
  }));
  createView($('local-detail')).render(fields.detail.filter(([key]) => ['sharpen', 'noise_luma', 'noise_color'].includes(key)).map(row => slider(['local', row[0]], row[1], row[2], row[3], row[4])));
  createView($('develop-curves')).render(Array.from({length: 4}, (_, channel) => <div key={channel} data-curve={channel} hidden={channel !== 0}>
    {Array.from({length: 5}, (_, point) => slider(['curves', channel, point], `${point * 25}%`, 0, 100, 1, 100))}
  </div>));
  createView($('develop-mixer')).render(Array.from({length: 8}, (_, color) => <div key={color} data-mixer={color} hidden={color !== 0}>
    {[t('色相'), t('彩度'), t('輝度')].map((label, index) => slider(['mixer', color, index], label, -100, 100))}
  </div>));
  createView($('develop-grading')).render(<>
    {Array.from({length: 4}, (_, zone) => <div key={zone} data-grading={zone} hidden={zone !== 0}>
      {slider(['grading', zone, 0], t('色相'), 0, 360)}{slider(['grading', zone, 1], t('彩度'), 0, 100)}{slider(['grading', zone, 2], t('輝度'), -100, 100)}
    </div>)}
    {slider(['grading_blending'], t('ブレンディング'), 0, 100)}{slider(['grading_balance'], t('バランス'), -100, 100)}
  </>);
  createView($('develop-crop')).render([t('左位置 %'), t('上位置 %'), t('幅 %'), t('高さ %')].map((label, index) => slider(['crop', index], label, index < 2 ? 0 : 1, index < 2 ? 99 : 100, .1, 100)));
  function chooseGroup(select: HTMLSelectElement, attribute: string) {
    for (const panel of document.querySelectorAll<HTMLElement>(`[data-${attribute}]`)) panel.hidden = panel.dataset[attribute] !== select.value;
    drawCurve();
  }
  for (const name of ['curve', 'mixer', 'grading']) $(`develop-${name}-channel`).onchange = event => chooseGroup(event.target as HTMLSelectElement, name);
  for (const button of document.querySelectorAll<HTMLButtonElement>('#curve-channel-buttons button')) button.onclick=()=>{
    $('develop-curve-channel').value=button.dataset.channel!;chooseGroup($('develop-curve-channel'),'curve');
  };
  function drawCurve() {
    if (!settings) return;
    toneCurve.draw();
    for(const button of document.querySelectorAll<HTMLButtonElement>('#curve-channel-buttons button'))button.setAttribute('aria-pressed',String(button.dataset.channel===$('develop-curve-channel').value));
  }
  function clippingIndicators(histogram: number[][] | null) {
    for (const [name, label, bin] of [['shadows', t("黒つぶれ"), 0], ['highlights', t("白飛び"), 255]] as const) {
      const indicator = $(`develop-clip-${name}`);
      const channels = histogram?.map(channel => channel[bin] > 0);
      const clipped = channels?.some(Boolean);
      indicator.style.color = clipped ? `rgb(${channels!.map(value => value ? 220 : 35).join(',')})` : '';
      const description = `${label}：${channels ? clipped ? t("あり") : t("なし") : t("未取得")}`;
      indicator.title = description;
      indicator.setAttribute('aria-label', description);
    }
  }
  function histogram(result: DevelopedPreview) {
    rawInfo = result.base.raw ?? null;
    rasterTemperature = result.base.raster_wb_temperature ?? null;
    whiteBalanceReady = true;
    const colorNote = result.base.color_note.split(' / ').map(part => t(part)).join(' / ');
    rawProfile.textContent = rawInfo ? t("プロファイル：{0}", colorNote.split(" / ").slice(1).join(" / ")) : "";
    sync();
    const maximum = Math.max(1, ...result.info.histogram.flat().map(value => Math.sqrt(value)));
    for (let channel = 0; channel < 3; channel++) {
      const points = result.info.histogram[channel].map((value, i) => `${i},${90 - Math.sqrt(value) / maximum * 85}`).join(' ');
      $(`develop-hist-${channel}`).setAttribute('points', `0,90 ${points} 255,90`);
    }
    const common = result.info.histogram[0].map((_, i) =>
      `${i},${90 - Math.sqrt(Math.min(...result.info.histogram.map(channel => channel[i]))) / maximum * 85}`).join(' ');
    $('develop-hist-common').setAttribute('points', `0,90 ${common} 255,90`);
    clippingIndicators(result.info.histogram);
    $('develop-histogram').setAttribute('title', t("{0}\n{1} × {2}・描画 {3} ms", colorNote, result.info.width, result.info.height, Math.round(result.info.elapsed_ms)));
  }
  function schedule() {
    pending = true; ++renderToken;
    if (!timer && !running) timer = setTimeout(() => {timer=undefined;render();}, 0);
  }
  async function render() {
    if (!active || !item || !settings || running || !pending) return;
    const flight = {gestureRender: interacting, bound};
    running = flight; pending = false;
    const token = renderToken, selection = epoch;
    const previewBound = bound, gestureRender = interacting;
    if (!saveFailed) status(interacting ? t("編集中") : t("原本を読み込み・現像中…"));
    try {
      const result = await invoke('develop_preview', { id: item.id, settings: tools.previewSettings(before ? defaults! : settings), bound: previewBound });
      const ongoingGesture = interacting && gestureRender;
      if (running === flight && active && selection === epoch && (token === renderToken || ongoingGesture)) {
        show(result); histogram(result); tools.update();
        const comparison=tools.comparison();
        if(comparison && !interacting){
          const other=await invoke('develop_preview',{id:item.id,settings:comparison.settings,bound});
          if(token!==renderToken||selection!==epoch)return;
          compare(other,comparison.mode);
        }else if (!comparison) compare(null);

        if (!saveFailed) status(before ? t("変更前を表示中") : interacting || dirty() ? t("編集中") : t("保存済み"));
      }
    } catch (failure) {
      if (active && token === renderToken && selection === epoch) { if (!saveFailed) status(t("現像プレビューを生成できません")); error(failure); }
    } finally {
      if (running === flight) {
        running = false;
        if (pending) timer = setTimeout(() => {timer=undefined;render();}, 0);
      }
    }
  }
  async function select(next: Photo | null | undefined) {
    clearTimeout(gestureTimer);
    clearTimeout(saveTimer);
    const selection = ++epoch;
    ++renderToken; pending = false;
    invoke('cancel_develop_preview').catch(error);
    tools.select(next?.id, settings); compare(null);
    rawInfo = null;
    rasterTemperature = null;
    whiteBalanceReady = false;
    $('full-image').hidden = true;
    $('develop-histogram').removeAttribute('title');
    for (const channel of [0, 1, 2, 'common']) $(`develop-hist-${channel}`).setAttribute('points','0,90 255,90');
    clippingIndicators(null);
    item = next ?? null; settings = baseline = null; undo = []; redo = []; gesture = null; gestureOwner = null; interacting = false; before = false; bound = 1600;
    pendingStep = null; historyRows = []; ++historyToken;
    historyView.render(null); $('develop-history-more').hidden = true;
    $('develop-history-empty').hidden = false;
    loading = Boolean(item); saveFailed = false; syncFailed = false; buttons();
    status(item ? t("現像設定を読み込み中…") : t("写真を選択してください"));
    for (const field of $('develop-photo-info').querySelectorAll<HTMLElement>(':scope > *')) { field.textContent = '—'; field.title = t("{0}：情報なし", t(field.dataset.photoField!)); }
    if (!item) return;
    photoInfo(item).then(fields => {
      if (selection !== epoch) return;
      for (const field of $('develop-photo-info').querySelectorAll<HTMLElement>(':scope > *')) {
        const name = field.dataset.photoField!;
        field.textContent = fields[name] ? (name === 'ISO' ? `ISO ${fields[name]}` : metadataValue(name, fields[name])) : '—';
        field.title = `${t(name)}: ${metadataValue(name, fields[name]) || t("情報なし")}`;
      }
    }).catch(() => {});
    try {
      const result = await invoke('get_develop', { id: item.id });
      if (selection !== epoch) return;
      settings = clone(result.settings); defaults = clone(result.defaults); baseline = clone(settings); revision = result.revision;
      undo = (result.undo || []).slice(-10).map(clone);
      loading = false; sync(); schedule(); refreshHistory();
    } catch (failure) { if (selection === epoch) { loading = false; status(t("現像設定を取得できません")); error(failure); buttons(); } }
  }
  function save() {
    clearTimeout(saveTimer);
    finishGesture();
    if (saveTask) return saveTask;
    if (!item || !settings || (!dirty() && !syncFailed)) return Promise.resolve(true);
    saving = true; saveFailed = false; buttons();
    const selection = epoch, id = item.id;
    // Coalesce edits made during a write, and let navigation await the latest revision.
    saveTask = Promise.resolve().then(async () => {
      try {
        while (selection === epoch && settings && dirty() && !interacting) {
          finishGesture();
          if (!pendingStep) recordStep(baseline, t("現像設定"));
          const step = pendingStep;
          const snapshot = clone(settings);
          const steps = [{settings: snapshot, label: pendingStep?.label || t("現像設定")}];
          const undoSnapshot = undo.slice(-10).map(clone);
          status(t("カタログに保存中…"));
          const nextRevision = await invoke('save_develop', { id, settings: snapshot, expectedRevision: revision, steps, undo: undoSnapshot });
          saved(id, false);
          if (selection !== epoch) return false;
          revision = nextRevision; baseline = snapshot; if (pendingStep === step) pendingStep = null;
        }
        syncFailed = true;
        await tools.afterSave(baseline!);
        syncFailed = false;
        status(interacting || dirty() ? t("編集中") : t("保存済み"));
        refreshHistory();
        return true;
      } catch (failure) {
        saveFailed = true;
        error(failure); status(t("保存できませんでした。編集内容は保持しています。「今すぐ保存」で再試行してください。"));
        return false;
      } finally { saving = false; saveTask = null; buttons(); if(dirty() && !saveFailed && !interacting) saveTimer=setTimeout(save,0); }
    });
    return saveTask;
  }
  async function refreshHistory(more = false) {
    if (!item) return;
    const token = ++historyToken, selection = epoch, id = item.id;
    try {
      const rows = await invoke('develop_history', { id, before: more ? historyRows.at(-1)?.id ?? null : null });
      if (token !== historyToken || selection !== epoch) return;
      historyRows = more ? historyRows.concat(rows) : rows;
      $('develop-history-more').hidden = true;
      historyView.render(historyRows.map(row => <button key={row.id} className="history-row" title={new Date(row.created).toLocaleString(language)} onClick={() => { void restoreHistory(row); }}>
        {row.label}<small>{new Date(row.created).toLocaleString(language)}</small>
      </button>));
      $('develop-history-empty').hidden = historyRows.length > 0;
    } catch (failure) { if (selection === epoch && token === historyToken) error(failure); }
  }
  async function restoreHistory(row: HistoryRow) {
    if (!item || !settings || loading || leaving || exporting) return;
    leaving = true; buttons();
    try {
      const restored = await invoke('history_settings', { id: item.id, historyId: row.id });
      if (!await save()) return;
      if($('history-action').value==='rename'){await tools.historyRename(row.id);await refreshHistory();return;}
      if($('history-action').value==='reference'){tools.reference(restored);return;}
      const previous = clone(settings); settings = restored;
      remember(previous, t("履歴から復元：{0}", row.label.replace(/^履歴から復元：/, '')));
      changed(); await save();
    } catch (failure) { error(failure); }
    finally { leaving = false; buttons(); }
  }
  $('develop-history-more').onclick = () => refreshHistory(true);
  $('develop-save').onclick = save;
  $('develop-reset').onclick = () => edit(() => { settings = clone(defaults); }, t("リセット"));
  $('develop-undo').onclick = () => {
    finishGesture(); if (!settings || !undo.length) return;
    const previous = clone(settings); redo.push(previous); settings = undo.pop()!; recordStep(previous, t("取り消し")); changed();
  };
  $('develop-redo').onclick = () => {
    finishGesture(); if (!settings || !redo.length) return;
    const previous = clone(settings); undo.push(previous); settings = redo.pop()!; recordStep(previous, t("やり直し")); changed();
  };
  async function copySettings(id: number) {
    if (loading || leaving || exporting) return;
    finishGesture();
    exporting = true; buttons();
    try {
      const source = active && item?.id === id && settings ? clone(settings) : (await invoke('get_develop', {id})).settings;
      const patch = await chooseCopySettings(source);
      if (patch) {
        clipboard = patch;
        $(active ? 'develop-status' : 'status').textContent = t('現像設定をコピーしました');
      }
    } catch (failure) { error(failure); }
    finally { exporting = false; buttons(); }
  }
  async function pasteSettings(id: number) {
    if (!clipboard || loading || leaving || exporting) return false;
    if (active) {
      if (item?.id !== id || !settings) return false;
      tools.paste(clipboard);
      exporting = true; buttons();
      try { return await save(); }
      finally { exporting = false; buttons(); }
    }
    exporting = true; buttons();
    let persisted = false;
    try {
      const recipe = await invoke('get_develop', {id});
      const next = Object.assign(clone(recipe.settings), clone(clipboard));
      await invoke('save_develop', {id, settings: next, expectedRevision: recipe.revision,
        steps: [{settings: next, label: t('設定を貼り付け')}], undo: [...(recipe.undo || []), recipe.settings].slice(-10)});
      persisted = true;
      saved(id);
      await invoke('refresh_library_preview', {id});
      saved(id);
      $('status').textContent = t('現像設定を貼り付けました');
      return true;
    } catch (failure) {
      error(persisted ? t('設定は保存済みですが、表示を更新できませんでした。{0}', failure) : failure);
      return false;
    } finally { exporting = false; buttons(); }
  }
  $('develop-copy').onclick = () => { if (item) void copySettings(item.id); };
  $('develop-paste').onclick = () => { if (item) void pasteSettings(item.id); };
  $('develop-before').onclick = () => { if (!settings) return; before = !before; buttons(); schedule(); };
  $('develop-mono').onchange = event => edit(value => { value.monochrome = (event.target as HTMLInputElement).checked; });
  for (const axis of ['h', 'v'] as const) $(`develop-flip-${axis}`).onchange = event => edit(value => { value[`flip_${axis}`] = (event.target as HTMLInputElement).checked; });
  $('develop-rotate').onclick = () => edit(value => { value.quarter_turns = (value.quarter_turns + 1) % 4; });
  $('develop-preset').onchange = event => {
    const preset = (event.target as HTMLSelectElement).value; if (!preset) return;
    edit(current => {
      if (preset === 'neutral') settings = clone(defaults);
      if (preset === 'mono') { current.monochrome = true; current.contrast = 20; }
      if (preset === 'vivid') { current.vibrance = 25; current.contrast = 12; current.sharpen = 30; }
    });
    (event.target as HTMLSelectElement).value = '';
  };
  document.addEventListener('keydown', event => {
    if ($('app-shell').inert || $('workspace').inert || document.querySelector('dialog[open]') || !$('photo-menu').hidden || document.body.dataset.catalogBusy) return;
    if (!active || !(event.metaKey || event.ctrlKey)) return;
    const key = event.key.toLowerCase();
    if (key === 's') { event.preventDefault(); save(); }
    if (key === 'z' && !(event.target as Element).matches('input,textarea')) { event.preventDefault(); $(event.shiftKey ? 'develop-redo' : 'develop-undo').click(); }
  });
  window.addEventListener('beforeunload', event => { if (dirty() || saving || exporting) { event.preventDefault(); event.returnValue = ''; } });
  buttons();
  return {
    get active() { return active; },
    get dirty() { return dirty(); },
    get busy() { return loading || saving || exporting || leaving || Boolean(gesture); },
    select,
    copySettings,
    pasteSettings,
    rotate(turns: number) { if (active && settings) edit(value => { value.quarter_turns = (value.quarter_turns + turns + 4) % 4; }); },
    async activate(value: boolean, next: Photo | null) {
      active = value;
      $('develop-panel').hidden = !value;
      $('develop-history-panel').hidden = !value;
      if (value) await select(next);
      else { tools.select(null,settings); compare(null); ++epoch; ++renderToken; pending = false; invoke('cancel_develop_preview').catch(error); }
    },
    async allowChange() {
      if (exporting || leaving) return false;
      leaving = true; buttons();
      try { return await save(); }
      finally { leaving = false; buttons(); }
    },
    highQuality() { if (active && settings) { bound = 0; schedule(); } },
  };
}
