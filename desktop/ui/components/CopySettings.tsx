import { useState } from 'react';
import { createView } from './view';
import { t } from '../i18n';
import type { Settings, SettingsPatch, SettingKey } from '../models';

interface CopyOption { label: string; fields: SettingKey[] }
interface CopyGroup { label: string; checked: boolean; options: CopyOption[] }
// Only editable settings are offered; camera profiles and unsupported tools are not recipes.
const groups: CopyGroup[] = [
  { label: '色表現', checked: true, options: [{ label: '白黒', fields: ['monochrome'] }] },
  { label: '基本補正', checked: true, options: [
    { label: 'ホワイトバランス', fields: ['temperature', 'tint', 'raw_temperature', 'raw_tint', 'raster_temperature', 'raster_tint', 'raster_reference_temperature'] },
    ...(['exposure', 'contrast', 'highlights', 'shadows', 'whites', 'blacks', 'texture', 'clarity', 'dehaze', 'vibrance', 'saturation'] as const)
      .map((key, index) => ({ label: ['露光量', 'コントラスト', 'ハイライト', 'シャドウ', '白レベル', '黒レベル', 'テクスチャ', '明瞭度', 'かすみの除去', '自然な彩度', '彩度'][index], fields: [key] })),
  ] },
  { label: 'カーブ', checked: true, options: [
    { label: 'パラメトリックカーブ', fields: ['curves'] }, { label: 'ポイントカーブ', fields: ['point_curves'] },
  ] },
  { label: 'カラーミキサー', checked: true, options: [{ label: 'HSL', fields: ['mixer'] }] },
  { label: 'カラーグレーディング', checked: true, options: [{ label: 'カラーグレーディング', fields: ['grading', 'grading_balance', 'grading_blending'] }] },
  { label: 'ディテール', checked: true, options: [
    { label: 'シャープ', fields: ['sharpen', 'sharpen_radius', 'sharpen_detail', 'sharpen_masking'] },
    { label: '輝度ノイズ低減', fields: ['noise_luma'] }, { label: 'カラーノイズ低減', fields: ['noise_color'] },
  ] },
  { label: 'レンズ補正', checked: true, options: [{ label: '手動の歪曲補正', fields: ['distortion'] }] },
  { label: '効果', checked: true, options: [
    { label: '周辺光量', fields: ['vignette', 'vignette_midpoint', 'vignette_feather'] },
    { label: '粒子', fields: ['grain', 'grain_size', 'grain_roughness'] },
  ] },
  { label: '変形', checked: false, options: [
    { label: '手動変形', fields: ['perspective_h', 'perspective_v', 'scale', 'aspect', 'offset_x', 'offset_y', 'constrain_crop'] },
  ] },
  { label: '切り抜き', checked: false, options: [
    { label: '角度・回転・反転', fields: ['angle', 'quarter_turns', 'flip_h', 'flip_v'] },
    { label: '切り抜き範囲', fields: ['crop'] },
  ] },
  { label: 'マスク', checked: false, options: [{ label: '部分補正', fields: ['masks'] }] },
];

function CopySettings({ settings, close }: { settings: Settings; close(patch: SettingsPatch | null): void }) {
  const [selected, setSelected] = useState(() => new Set(groups.filter(group => group.checked).flatMap(group => group.options.flatMap(option => option.fields))));
  function toggle(fields: SettingKey[], checked: boolean) {
    setSelected(previous => {
      const next = new Set(previous);
      for (const field of fields) { if (checked) next.add(field); else next.delete(field); }
      return next;
    });
  }
  return <form onSubmit={event => {
    event.preventDefault();
    if (selected.size) close(Object.fromEntries([...selected].map(key => [key, structuredClone(settings[key])])));
  }}>
    <h2 id="copy-settings-title">{t('設定をコピー')}</h2>
    <div className="copy-settings-groups">{groups.map(group => {
      const fields = group.options.flatMap(option => option.fields);
      const count = fields.filter(field => selected.has(field)).length;
      return <fieldset key={group.label}>
        <legend><label><input type="checkbox" checked={count === fields.length}
          ref={input => { if (input) input.indeterminate = count > 0 && count < fields.length; }}
          onChange={event => toggle(fields, event.target.checked)} />{t(group.label)}</label></legend>
        {group.options.map(option => <label key={option.label}><input type="checkbox" data-copy-field={option.fields[0]}
          checked={option.fields.every(field => selected.has(field))}
          onChange={event => toggle(option.fields, event.target.checked)} />{t(option.label)}</label>)}
      </fieldset>;
    })}</div>
    <div className="copy-settings-actions">
      <button type="button" onClick={() => setSelected(new Set(groups.flatMap(group => group.options.flatMap(option => option.fields))))}>{t('すべてをチェック')}</button>
      <button type="button" onClick={() => setSelected(new Set())}>{t('チェックしない')}</button>
      <button type="button" onClick={() => setSelected(new Set(groups.filter(group => group.checked).flatMap(group => group.options.flatMap(option => option.fields))))}>{t('初期設定')}</button>
      <span />
      <button type="button" onClick={() => close(null)}>{t('キャンセル')}</button>
      <button id="copy-settings-confirm" type="submit" disabled={!selected.size}>{t('コピー')}</button>
    </div>
  </form>;
}

export function createCopySettings() {
  const dialog = document.createElement('dialog');
  dialog.id = 'copy-settings-dialog';
  dialog.setAttribute('aria-labelledby', 'copy-settings-title');
  document.body.append(dialog);
  const view = createView(dialog);
  return (settings: Settings) => new Promise<SettingsPatch | null>(resolve => {
    let result: SettingsPatch | null = null;
    dialog.addEventListener('close', () => { view.render(null); resolve(result); }, { once: true });
    view.render(<CopySettings settings={settings} close={patch => { result = patch; dialog.close(); }} />);
    dialog.showModal();
  });
}
