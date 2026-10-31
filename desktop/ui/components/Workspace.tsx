export function Workspace() {
  return <>
  <div id="app-shell" inert>
  <button id="toggle-modulebar" aria-label="モジュールバーを切り替え" aria-expanded="true" title="モジュールバー (F5)">▴</button>
  <header id="module-bar" className="module-bar">
    <div className="identity"><img className="app-mark" src="branding/openroom-icon-small.png" width="40" height="40" alt="" /><div><small>OpenRoom</small></div></div>
    <nav aria-label="モジュール">
      <button id="module-library" className="module active" aria-current="page">ライブラリ</button>
      <button id="module-develop" className="module" title="現像 (D)">現像</button>
    </nav>
  </header>

  <div className="workspace-shell">
    <button id="toggle-sidebar" className="panel-edge" aria-label="左パネルを切り替え" aria-expanded="true">◂</button>
    <div id="workspace" className="workspace">
      <aside id="sidebar" className="sidebar" aria-label="ライブラリとフォルダ">
        <details id="navigator-panel" className="navigator panel" open>
          <summary className="panel-heading"><h2>ナビゲーター</h2><div className="navigator-zoom"><button id="navigator-fit" className="active">全体</button><button id="navigator-100">100%</button><button id="navigator-50">50%</button></div></summary>
          <div id="navigator-preview" className="navigator-preview"><img id="navigator-image" alt="選択した写真" hidden /><div id="navigator-viewport" aria-hidden="true" hidden></div><span id="navigator-empty">写真を選択してください</span></div>
          <div id="navigator-resize" className="panel-resize" role="separator" tabIndex={0} aria-label="ナビゲーターの高さ" aria-orientation="horizontal" aria-controls="navigator-preview" title="ドラッグまたは上下キーで高さを調整"></div>
        </details>
        <div className="sidebar-scroll">
          <details className="panel catalog-panel">
            <summary>カタログ</summary>
            <div className="catalog-identity"><span className="catalog-icon" aria-hidden="true">▤</span><div><span className="catalog-caption">現在のカタログ</span><strong id="catalog-name"></strong></div></div>
            <button id="all" className="folder-row active"><span>すべての写真</span><span id="catalog-count" className="folder-count"></span></button>
            <details id="catalog-manage" className="catalog-manage">
              <summary>カタログを管理</summary>
              <div className="catalog-actions">
                <button id="catalog-open">開く</button><button id="catalog-new">新規作成</button>
                <button id="catalog-location" title="カタログの保存場所を開く">保存場所を表示</button>
                <button id="catalog-copy" title="カタログと付属プレビューをコピー。原本と現像用キャッシュは含めません">コピーを保存</button>
              </div>
              <p className="panel-note">カタログを切り替えると、編集を保存して再起動します。</p>
            </details>
          </details>
          <fieldset id="develop-left-controls" className="develop-left"><label className="preset-picker">プリセット<select id="develop-preset"><option value="">選択…</option><option value="neutral">初期設定</option><option value="mono">モノクロ・コントラスト</option><option value="vivid">鮮やか</option></select></label><div id="develop-left-panels"></div></fieldset>
          <details id="develop-history-panel" className="panel" open hidden><summary>ヒストリー</summary><div id="develop-history-list"></div><p id="develop-history-empty" className="panel-note">編集履歴はまだありません。</p><button id="develop-history-more" hidden>以前の履歴を表示</button></details>
          <details id="folder-panel" className="panel folder-panel" open>
            <summary className="panel-heading"><h2>フォルダー</h2><button id="add-small" className="small-icon" aria-label="フォルダを追加"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14M12 5v14" /></svg></button></summary>
            <div id="folder-content">
            <label className="folder-search"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" /></svg><input id="folder-search" type="search" aria-label="フォルダーをフィルタリング" placeholder="フォルダーをフィルタリング" /></label>
            <div id="roots"></div>
            <p id="no-folders" className="panel-note" hidden>写真フォルダを読み込んでください。</p>
            </div>
            <div id="folder-resize" className="panel-resize" role="separator" tabIndex={0} aria-label="フォルダーの高さ" aria-orientation="horizontal" aria-controls="folder-content" title="ドラッグまたは上下キーで高さを調整"></div>
          </details>
        </div>
        <div className="import-actions"><button id="add">読み込み…</button></div>
      </aside>
      <div id="sidebar-resize" className="panel-resize sidebar-resize" role="separator" tabIndex={0} aria-label="左パネルの幅" aria-orientation="vertical" aria-controls="sidebar" title="ドラッグまたは左右キーで幅を調整"></div>

      <main className="library-main">
        <section id="filter-panel" className="filter-panel" aria-label="ライブラリフィルター">
          <div className="filter-heading"></div><div id="filter-text"><label className="search"><input id="search" type="search" placeholder="名前・タグ・メモを検索" aria-label="名前・タグ・メモを検索" autoComplete="off" /><kbd>⌘ F</kbd></label><button id="scan" title="フォルダを再走査">↻ 再読み込み</button></div><div id="attribute-filters"></div><div id="metadata-filters"></div>
        </section>
        <div id="progress" role="status" hidden><span id="progress-text"></span><button id="cancel">中止</button></div>
        <div id="error" role="alert" hidden></div>
        <div id="ai-selection" hidden>AI選択 <button id="ai-selection-clear">絞り込みを解除</button></div>
        <div className="photo-stage">
          <section id="viewport" aria-label="写真一覧" tabIndex={0} hidden><div id="grid" role="group"></div></section>
          <section id="viewer" aria-label="ルーペ表示" tabIndex={0}>
            <svg id="develop-overlay" tabIndex={-1} aria-label="写真上の編集範囲"></svg><div id="image-area"><img id="compare-image" alt="比較基準" hidden /><img id="full-image" crossOrigin="anonymous" alt="" hidden /><p id="view-message" role="status" hidden></p></div>
          </section>
          <section id="review-view" aria-label="写真の比較・選別" hidden>
            <div className="review-toolbar"><p id="review-message" role="status"></p><div id="compare-controls"><button id="compare-swap">入れ替え</button><button id="compare-promote">候補を基準に</button><label><input id="compare-linked" type="checkbox" defaultChecked />拡大・移動を連動</label></div></div>
            <div id="review-photos"></div>
          </section>
          <div id="empty" hidden><h1 id="empty-title">写真フォルダを読み込んでください</h1><p id="empty-description">原本を移動せず、写真をカタログで管理できます。</p><button id="empty-add">読み込み…</button></div>
        </div>
        <div className="view-tools" aria-label="表示ツール">
          <div className="view-modes library-tool"><button id="grid-view" className="tool" aria-label="グリッド表示" aria-pressed="false" title="グリッド表示 (G)"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="1" /><path d="M9 4v16M15 4v16M3 12h18" /></svg></button><button id="loupe-view" className="tool active" aria-label="ルーペ表示" aria-pressed="true" title="ルーペ表示 (E)"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="1" /><rect x="6" y="7" width="12" height="10" /></svg></button><button id="compare-view" className="tool" aria-label="比較表示" aria-pressed="false" title="比較表示 (C)"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="2" y="5" width="8" height="14" rx="1" /><rect x="14" y="5" width="8" height="14" rx="1" /><path d="M12 3v18" /></svg></button><button id="survey-view" className="tool" aria-label="選別表示" aria-pressed="false" title="選別表示 (N)"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="8" height="7" rx="1" /><rect x="14" y="4" width="7" height="7" rx="1" /><rect x="6" y="14" width="12" height="6" rx="1" /></svg></button></div>
          <span className="tool-divider library-tool"></span>
          <div className="library-tool" id="photo-flags" aria-label="選択した写真のフラグ"><button data-flag="1" title="採用 (P)" aria-label="採用"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M5 21V3m0 1c5-4 9 4 14 0v10c-5 4-9-4-14 0" /></svg></button><button data-flag="-1" title="除外 (X)" aria-label="除外"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 21V3m0 1c5-4 9 4 14 0v7M14 14l7 7m0-7-7 7" /></svg></button></div><select className="library-tool" id="photo-label" aria-label="カラーラベル"><option value="">ラベルなし</option><option value="red">レッド</option><option value="yellow">イエロー</option><option value="green">グリーン</option><option value="blue">ブルー</option><option value="purple">パープル</option></select><div id="rating-stars" className="rating-stars library-tool" aria-label="選択した写真の評価"><button data-rating="1" aria-label="評価1"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z" /></svg></button><button data-rating="2" aria-label="評価2"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z" /></svg></button><button data-rating="3" aria-label="評価3"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z" /></svg></button><button data-rating="4" aria-label="評価4"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z" /></svg></button><button data-rating="5" aria-label="評価5"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z" /></svg></button></div>
          <span className="tool-divider library-tool"></span>
          <div id="develop-view-tools" className="develop-actions"><button id="develop-before" aria-pressed="false" title="変更前と比較">変更前と比較</button><button id="develop-undo" aria-label="取り消し" title="取り消し"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m8 4-5 5 5 5M3 9h10a7 7 0 0 1 7 7v4" /></svg></button><button id="develop-redo" aria-label="やり直し" title="やり直し"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m16 4 5 5-5 5m5-5H11a7 7 0 0 0-7 7v4" /></svg></button></div>
          <div id="loupe-tools" className="loupe-tools"><button id="zoom-out" aria-label="縮小"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14" /></svg></button><span id="zoom-level" aria-label="表示倍率"></span><button id="zoom-in" aria-label="拡大"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14M12 5v14" /></svg></button><button id="high-quality" title="現像画面を開いて原本を読み込む">現像で原本を表示</button></div>
          <div id="photo-rotation" className="loupe-tools" aria-label="写真の回転"><button id="photo-rotate-left" aria-label="反時計回りに90度回転" title="反時計回りに90度回転"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 10a8 8 0 1 1 1 7M4 4v6h6" /></svg></button><button id="photo-rotate-right" aria-label="時計回りに90度回転" title="時計回りに90度回転"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 10a8 8 0 1 0-1 7M20 4v6h-6" /></svg></button></div>
          <div id="grid-tools" className="grid-tools" hidden><select id="sort" aria-label="並び順"><option value="name">名前順</option><option value="added">登録日順</option><option value="modified">更新日順</option><option value="size">サイズ順</option></select><button id="descending" aria-label="昇順・降順" aria-pressed="false">↑</button><label className="thumbnail-size">サムネール<input id="tile-size" type="range" min="150" max="500" defaultValue="400" step="10" aria-label="サムネイルの大きさ" /></label></div>
          <div className="develop-save-tools"><p id="develop-status" role="status">写真を選択してください</p><button id="develop-save">今すぐ保存</button></div>
          <button id="toggle-filters" className="toolbar-end" aria-label="ライブラリフィルターを切り替え" aria-expanded="false" title={"ライブラリフィルター (\\\\)"}><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" /></svg></button>
        </div>
      </main>

      <aside id="develop-panel" className="develop-panel" aria-label="現像設定" hidden>
        <div className="develop-scroll">
        <details className="panel histogram-panel" open><summary>ヒストグラム</summary>
          <div className="histogram-frame">
            <div className="histogram-plot">
              <svg id="develop-histogram" viewBox="0 0 255 90" preserveAspectRatio="none" aria-label="RGBヒストグラム" role="img"><polygon id="develop-hist-0" /><polygon id="develop-hist-1" /><polygon id="develop-hist-2" /><polygon id="develop-hist-common" /></svg>
              <span id="develop-clip-shadows" className="histogram-clip" role="img" aria-label="黒つぶれ：未取得" title="黒つぶれ：未取得">▲</span>
              <span id="develop-clip-highlights" className="histogram-clip" role="img" aria-label="白飛び：未取得" title="白飛び：未取得">▲</span>
            </div>
            <div id="develop-photo-info" aria-label="撮影情報"><span data-photo-field="ISO">—</span><span data-photo-field="焦点距離">—</span><span data-photo-field="絞り">—</span><span data-photo-field="シャッター速度">—</span></div>
          </div>
        </details>
        <div id="develop-toolstrip" className="develop-toolstrip"></div>
        <fieldset id="develop-controls" disabled>
          <div id="develop-tools-layout"></div>
          <details className="panel" open><summary>基本補正</summary><label className="develop-check"><input id="develop-mono" type="checkbox" />白黒</label><div id="develop-basic" className="develop-group"></div></details>
          <details id="tone-curve-panel" className="panel" open><summary>トーンカーブ</summary><div id="curve-channel-buttons" role="group" aria-label="トーンカーブのチャンネル"><button data-channel="0" aria-label="RGBカーブ">●</button><button data-channel="1" aria-label="赤カーブ">●</button><button data-channel="2" aria-label="緑カーブ">●</button><button data-channel="3" aria-label="青カーブ">●</button></div><select hidden id="develop-curve-channel" aria-label="カーブのチャンネル"><option value="0">RGB</option><option value="1">レッド</option><option value="2">グリーン</option><option value="3">ブルー</option></select><svg className="tone-curve" viewBox="0 0 256 256" tabIndex={0} role="application" aria-label="ポイントトーンカーブ。クリックで追加、ドラッグで調整、Deleteで削除"><path d="M64 0V256M128 0V256M192 0V256M0 64H256M0 128H256M0 192H256" /><polyline id="develop-curve-line" /><g id="curve-handles"></g></svg><div id="curve-point-controls"><label>入力<input id="curve-input" type="number" min="0" max="255" /></label><label>出力<input id="curve-output" type="number" min="0" max="255" /></label><button id="curve-delete">点を削除</button><button id="curve-reset">直線に戻す</button></div><div id="develop-curves" className="develop-group" hidden></div></details>
          <details className="panel"><summary>カラーミキサー</summary><select id="develop-mixer-channel" aria-label="ミキサーの色"><option value="0">レッド</option><option value="1">オレンジ</option><option value="2">イエロー</option><option value="3">グリーン</option><option value="4">アクア</option><option value="5">ブルー</option><option value="6">パープル</option><option value="7">マゼンタ</option></select><div id="develop-mixer" className="develop-group"></div></details>
          <details id="color-grading-panel" className="panel"><summary>カラーグレーディング</summary><div id="develop-grading"></div></details>
          <details className="panel"><summary>ディテール</summary><div id="develop-detail" className="develop-group"></div><p className="panel-note">通常のエッジ・ノイズ補正。AIノイズ除去は後続実装です。</p></details>
          <details id="lens-correction-panel" className="panel"><summary>レンズ補正</summary>
            <label className="develop-check"><input id="develop-ca-auto" type="checkbox" />色収差を自動補正</label>
            <p id="develop-ca-status" className="panel-note"></p>
            <div id="develop-lens" className="develop-group"></div>
            <label className="develop-check"><input id="develop-defringe" type="checkbox" />フリンジを除去</label>
            <div id="develop-fringe" className="develop-group"></div>
            <p className="panel-note">効果は100%以上に拡大して確認してください。</p>
          </details>
          <details className="panel"><summary>変形</summary><div id="develop-geometry" className="develop-group"></div></details>
          <details className="panel"><summary>効果</summary><div id="develop-effects" className="develop-group"></div></details>
          <div className="develop-actions"><button id="develop-copy">設定をコピー</button><button id="develop-paste">貼り付け</button></div>
        </fieldset>
        </div>
        <fieldset id="mask-palette" hidden aria-label="マスクレイヤー"></fieldset><div className="develop-bottom"><div className="develop-actions"><button id="develop-previous" title="前の写真の現像設定を適用">前の設定</button><button id="develop-reset">リセット</button></div></div>
      </aside>
      <aside id="library-inspector" className="library-inspector" aria-label="選択した写真の詳細" hidden>
        <details className="panel" open><summary>クイック情報</summary><div id="selection-empty" className="panel-note">写真を選択してください。</div><div id="selection-content" hidden><h2 id="selection-name"></h2><form id="annotation-form"><label>評価<select id="annotation-rating"><option value="0">未評価</option><option value="1">★</option><option value="2">★★</option><option value="3">★★★</option><option value="4">★★★★</option><option value="5">★★★★★</option></select></label><label>キーワード<input id="annotation-tags" maxLength={25600} placeholder="旅行, 作品候補" /></label><label>メモ<textarea id="annotation-memo" rows={3} maxLength={65536}></textarea></label><button id="annotation-save" type="submit">保存</button><span id="annotation-status" role="status"></span></form></div></details>
        <details className="panel" open><summary>メタデータ</summary><p id="photo-status" className="panel-note"></p><dl id="photo-details"></dl></details>
        <details className="panel"><summary>ファイル情報</summary><dl id="selection-details"></dl><button id="selection-reveal" className="reveal-button">保存場所を開く</button></details>
      </aside>
    </div>
    <button id="toggle-inspector" className="panel-edge" aria-label="右パネルを切り替え" aria-expanded="false">◂</button>
  </div>

  <div className="filmstrip-bar"><div className="filmstrip-navigation"><button id="film-grid" aria-label="グリッド表示に切り替え"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="1" /><path d="M9 4v16M15 4v16M3 12h18" /></svg></button><button id="film-previous" aria-label="前の写真へ"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m10 5-7 7 7 7M3 12h18" /></svg></button><button id="film-next" aria-label="次の写真へ"><svg className="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m14 5 7 7-7 7M21 12H3" /></svg></button></div><h1 id="title">すべての写真</h1><span id="result-count">カタログを読み込み中</span><span id="selection-count"></span><span id="view-title"></span><label className="filmstrip-filter">フィルター：<select id="rating-filter" aria-label="評価で絞り込み"><option value="">フィルターオフ</option><option value="0">未評価</option><option value="1">★</option><option value="2">★★</option><option value="3">★★★</option><option value="4">★★★★</option><option value="5">★★★★★</option></select></label></div>
  <section id="filmstrip" aria-label="フィルムストリップ" tabIndex={0}><div id="filmstrip-track"></div></section>
  <footer><span id="status">準備中</span><button id="toggle-filmstrip" aria-label="フィルムストリップを切り替え" aria-expanded="true">▾</button></footer>
  <dialog id="discard-dialog" aria-labelledby="discard-title" aria-describedby="discard-message"><form method="dialog"><h2 id="discard-title">未保存の変更</h2><p id="discard-message"></p><div><button value="cancel" autoFocus>キャンセル</button><button value="discard">変更を破棄</button></div></form></dialog>
  </div>
  </>;
}
