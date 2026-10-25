import { createPortal } from 'react-dom';
import { t } from '../i18n';
import { element as $ } from '../dom';

export function DevelopToolsLayout() {
  return <>
<details className="panel tool-options-panel" id="crop-tool-panel"><summary id="photo-tool-heading">{t("切り抜きと角度補正")}</summary><div className="tool-options">
      <label id="photo-tool-picker">{t("ツール")}<select id="photo-tool"><option value="">{t("通常表示")}</option><option value="crop">{t("切り抜きと角度補正")}</option><option value="straighten">{t("水平線を指定")}</option><option value="brush">{t("ブラシ")}</option><option value="linear">{t("線形マスク")}</option><option value="radial">{t("円形マスク")}</option><option value="color">{t("色域マスク")}</option><option value="luminance">{t("輝度域マスク")}</option></select></label>
      <div id="crop-options" className="crop-options">
        <div className="crop-aspect-row"><button id="crop-orientation" className="crop-icon" aria-label={t("切り抜きの縦横を入れ替え")} title={t("縦横を入れ替え")}>⇄</button><label className="crop-ratio-row">{t("縦横比：")}<select id="crop-ratio" defaultValue="original"><option value="original">{t("元画像")}</option><option value="0">{t("自由")}</option><option value="custom" hidden>{t("カスタム")}</option><optgroup label={t("基本比率")}><option value="1">1:1</option><option value="1.5">3:2</option><option value="1.3333333333333333">4:3</option><option value="1.25">5:4</option><option value="1.7777777777777777">16:9</option></optgroup><optgroup label={t("用紙サイズの比率")}><option value="1.4142135623730951">{t("A判・B判（1:√2）")}</option><option value="1.4269662921348314">{t("L判")} 89×127 mm</option><option value="1.4015748031496063">{t("2L判")} 127×178 mm</option><option value="1.4901960784313726">KG 102×152 mm</option><option value="1.251231527093596">{t("六つ切")} 203×254 mm</option><option value="1.5024630541871922">{t("六つ切ワイド")} 203×305 mm</option><option value="1.2007874015748032">{t("四つ切")} 254×305 mm</option><option value="1.4409448818897639">{t("四つ切ワイド")} 254×366 mm</option></optgroup></select></label><button id="crop-lock" className="crop-icon" aria-label={t("縦横比を固定")} aria-pressed="true" title={t("縦横比を固定")}><svg viewBox="0 0 24 24" className="ui-icon" aria-hidden="true"><path className="lock-shackle" d="M7 10V7a5 5 0 0 1 10 0v3" /><rect x="5" y="10" width="14" height="11" rx="2" /></svg></button></div>
      </div>
      <div id="rotation-options">
        <div className="crop-auto-row"><button id="crop-auto">{t("自動")}</button></div>
        <div className="crop-angle-row"><button id="crop-straighten" className="crop-icon" aria-label={t("水平線を指定")} title={t("写真の水平にしたい線をドラッグ")} aria-pressed="false"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 8h18v10H3zM7 8v4M12 8v6M17 8v4" /></svg></button><div id="straighten-adjustments" className="develop-group" /></div>
        <p id="straighten-help" className="panel-note" hidden>{t("写真の水平にしたい線に沿ってドラッグします。")}</p>
        <p id="crop-feedback" className="panel-note" role="status" hidden />
        <label className="crop-constrain"><input id="constrain-crop" type="checkbox" />{t("画像に固定")}</label>
        <label className="crop-overlay-row">{t("ツールオーバーレイ：")}<select id="crop-overlay" defaultValue="always"><option value="always">{t("常にオン")}</option><option value="auto">{t("自動")}</option><option value="never">{t("常にオフ")}</option></select></label>
        <details id="crop-advanced"><summary>{t("詳細")}</summary>
          <div className="crop-orientation" role="group" aria-label={t("切り抜きの向き")}><button id="crop-landscape" aria-pressed="false">{t("横長")}</button><button id="crop-portrait" aria-pressed="false">{t("縦長")}</button></div>
          <div className="develop-actions"><button id="develop-rotate">{t("90°回転")}</button><label><input id="develop-flip-h" type="checkbox" />{t("左右反転")}</label><label><input id="develop-flip-v" type="checkbox" />{t("上下反転")}</label></div>
          <div id="develop-crop" className="develop-group" /><button id="crop-clear">{t("切り抜きを解除")}</button>
        </details>
      </div>
      <div className="crop-footer"><button id="crop-reset">{t("リセット")}</button><button id="tool-done">{t("閉じる")}</button></div>
    </div></details><details className="panel mask-panel tool-options-panel" id="mask-tool-panel"><summary>{t("マスクの補正")}</summary><div className="tool-options">



      <p id="mask-draw-help" className="panel-note" />


      <label>{t("名前")}<input id="mask-name" maxLength={80} /></label>

      <label id="mask-radius-row">{t("ブラシサイズ")}<input id="mask-radius" type="range" min="0.005" max="0.5" step="0.005" defaultValue="0.06" /></label>
      <label id="mask-feather-row">{t("境界のぼかし")}<input id="mask-feather" type="range" min="0" max="1" step="0.01" defaultValue="0.5" /></label>
      <label>{t("効果量")}<input id="mask-amount" type="range" min="0" max="1" step="0.01" defaultValue="1" /></label>
      <h3 id="local-heading" className="adjustment-heading">{t("マスクの基本補正")}</h3>
      <p className="panel-note">{t("以下の補正は選択中のマスクだけに適用します。色温度・色かぶりは全体のWBに対する相対補正です。")}</p>
      <label><input id="local-monochrome" type="checkbox" />{t("白黒")}</label><div id="local-adjustments" />
      <details><summary>{t("詳細補正")}</summary><div id="local-detail" /></details>
      <div className="develop-actions"><button id="mask-reset">{t("補正をリセット")}</button><button id="mask-done">{t("閉じる")}</button></div>
    </div></details>{createPortal(<><details className="panel" id="presets-panel"><summary>{t("プリセット・同期")}</summary><div className="tool-options">
      <div id="copy-fields" /><button id="fields-modified">{t("変更済みのグループを選択")}</button>
      <input id="recipe-name" placeholder={t("プリセット・スナップショット名")} maxLength={80} aria-label={t("保存名")} />
      <select id="user-presets" aria-label={t("ユーザープリセット")} />
      <div className="develop-actions"><button id="preset-save">{t("保存")}</button><button id="preset-apply">{t("適用")}</button><button id="preset-rename">{t("名前変更")}</button><button id="preset-delete">{t("削除")}</button></div>
      <div className="develop-actions"><button id="preset-import">{t("読み込み…")}</button><button id="preset-export">{t("書き出し…")}</button></div>
      <button id="develop-sync">{t("選択写真に同期")}</button><label><input id="auto-sync" type="checkbox" />{t("自動同期（選択した補正項目）")}</label>
    </div></details><details className="panel" id="snapshots-panel"><summary>{t("比較・スナップショット")}</summary><div className="tool-options">
      <label>{t("比較表示")}<select id="compare-mode"><option value="">{t("なし")}</option><option value="side">{t("左右に比較")}</option><option value="split">{t("分割して比較")}</option></select></label>
      <button id="compare-current">{t("現在の状態を比較基準に")}</button><button id="compare-default">{t("初期状態を比較基準に")}</button>
      <label>{t("履歴クリック時")}<select id="history-action"><option value="restore">{t("復元")}</option><option value="reference">{t("比較基準に指定")}</option><option value="rename">{t("名前変更")}</option></select></label>
      <select id="snapshot-list" aria-label={t("スナップショット")} /><div className="develop-actions"><button id="snapshot-save">{t("保存")}</button><button id="snapshot-restore">{t("復元")}</button><button id="snapshot-compare">{t("比較")}</button><button id="snapshot-rename">{t("名前変更")}</button><button id="snapshot-delete">{t("削除")}</button></div>
    </div></details></>, $('develop-left-panels'))}
{createPortal(<details className="panel" open><summary>{t('マスク')}</summary><div className="tool-options"><button id="mask-new" aria-expanded="false">{t("新しいマスクを作成")}</button><div id="mask-types" className="mask-types" hidden>
        <button data-mask-type="brush">{t("ブラシ")}</button>
        <button data-mask-type="linear">{t("線形グラデーション")}</button>
        <button data-mask-type="radial">{t("円形グラデーション")}</button>
      </div><div id="mask-layers" role="group" aria-label={t("マスクレイヤー")} /><div id="mask-layer-actions" className="develop-actions"><button id="mask-up" title={t("上のレイヤーへ")}>↑</button><button id="mask-down" title={t("下のレイヤーへ")}>↓</button><button id="mask-copy">{t("複製")}</button><button id="mask-delete">{t("削除")}</button></div><label id="mask-overlay-row"><input id="mask-overlay" type="checkbox" defaultChecked />{t("オーバーレイを表示")}</label></div></details>, $('mask-palette'))}
</>;
}
