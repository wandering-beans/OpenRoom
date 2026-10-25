# AI連携（MCP）

[利用ガイドに戻る](README.md)


起動中のOpenRoomを、同じPCのMCPクライアントから操作できます。macOSの「OpenRoom → 環境設定…」（⌘,）を開き、AI連携を有効にして「設定を適用」を押します。初期状態は無効です。接続方式はStreamable HTTP、標準URLは`http://127.0.0.1:39127/mcp`です。クライアントには、このURLと`Authorization: Bearer <設定画面でコピーしたトークン>`を指定してください。stdioとOAuthには対応していません。

有効・無効とポートはアプリ設定に、トークンはOSの資格情報ストアに保存します。次回は画面とカタログの準備後に自動起動します。ポートの競合や資格情報ストアの失敗は設定画面に表示し、接続の受付を始めません。LinuxではSecret Serviceが利用できるデスクトップ環境が必要です。トークンはカタログのコピーに含めません。設定画面の「再発行」で旧トークンを無効にできます。無効化・終了・カタログの再起動では受付を止め、進行中のMCPジョブを中止します。

接続すると、開いているカタログへの保存・書き出しを確認なしで実行できます。localhostだけで待ち受け、Bearer認証とHost・Originを検証します。原本の変更・削除、既存ファイルの上書き、カタログの作成・切り替えを行うツールは公開していません。

| 用途 | ツール |
| --- | --- |
| 状態・画面 | `get_app_state`、`select_photos`、`set_view` |
| 検索・詳細 | `list_roots`、`list_folders`、`search_photos`、`count_photos`、`get_photo`、`get_metadata`、`get_metadata_facets` |
| 取り込み | `add_root`、`start_scan`、`start_metadata_index` |
| 整理 | `update_photo_attributes`、`update_annotations` |
| 現像 | `get_develop_settings`、`preview_photo`、`update_develop_settings` |
| プリセット・履歴 | `list_presets`、`save_preset`、`apply_preset`、`list_develop_history`、`restore_develop_history` |
| 書き出し・進捗 | `start_export`、`get_job`、`cancel_job` |

入力スキーマと説明は`tools/list`で取得できます。検索は1ページ最大200枚で、続きは同じ`query`と返された`next`を`cursor`として渡します。`select_photos`は最大200個のIDを受け取り、読み込み済みページの外にある写真も「AI選択」として表示します。先頭のIDが主選択です。画面の「絞り込みを解除」で通常の一覧へ戻せます。`set_view`は`grid`・`loupe`・`compare`・`survey`・`develop`に対応します。

たとえば、`search_photos`で探し、`preview_photo`の画像を確認してから、`select_photos`、`update_annotations`、`update_develop_settings`、`start_export`の順に操作できます。現像更新では、直前の`get_develop_settings`から得た`revision`を`expected_revision`に指定し、`patch`へ変更する項目だけを渡します。競合時は保存しません。プリセット適用と履歴復元にも現在のrevisionが必要で、どちらも新しい履歴を残します。

読み取りは保存済みデータが対象です。現像の`patch`と評価・タグ・メモの更新では、省略した項目を保持します。配列はフィールド全体の置換です。`preview_photo`に渡す一時的な`patch`は保存せず、JPEGをMCPの画像コンテンツとして返します。`get_app_state`では選択・表示・検索条件と未保存状態も取得できます。

変更と画面操作は直列に処理し、実行前に現像の自動保存を完了させます。未保存のタグ・メモ、保存失敗、モーダル表示中は内容を保持してエラーを返します。画面要求は30秒以内に実行を開始しなければ失効します。変更の保存後に画面更新だけが失敗した場合は、結果に`ui_updated: false`と`ui_error`を返します。応答を失った場合は保存済み状態を確認し、同じ変更を無条件に再送しないでください。

走査・メタデータ索引・一括書き出しはジョブIDを返します。`get_job`で完了まで確認し、結果の写真ごとのエラーも確認してください。索引作成は`query`のルート・フォルダ範囲が対象で、他の絞り込み条件は使いません。書き出しは開始時に対象と現像設定を固定し、指定した既存の絶対パスのディレクトリへ`<元の名前>-<ID>-edited.<拡張子>`で保存します。同名ファイルがあれば、その写真は失敗として返します。中止までに保存した編集や出力は残ります。ジョブの記録は起動中に最大100件保持します。

MCPの通信には[公式Rust SDK rmcp 3.6.0](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.6.0)を使っています。
