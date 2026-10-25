# 開発ガイド

[利用ガイドに戻る](README.md)

## ビルドと起動
Rust stable、Node.js 22.12以上と対象OSのビルド環境が必要です。デスクトップはTauri 2＋React＋TypeScript＋Viteです。ライブラリの未編集プレビューはmacOSでSwiftとImageIO/Core Imageを使用します。macOSの現像はrtengineとRustの組み合わせで、RAWには固定したrtengineを使います。Windows/LinuxのRAWは従来どおりrawler、HEIF/AVIFは全OSでlibheifを使います。

フロントエンドのソースは`desktop/ui/`、Viteの生成物は`desktop/ui-dist/`、Rustのバックエンドは`desktop/src/`に分離しています。フロントからのネイティブ呼び出しは`desktop/ui/backend.ts`を通します。フロント全体をTypeScript／TSXで記述し、厳格な型チェックを行います。写真一覧・現像操作・履歴・マスク・比較・設定・書き出しはReactで描画し、保存や画像の位置計算は型付きコントローラーが扱います。

依存の導入は`npm ci --prefix desktop`、型チェックとフロントのビルドは`npm --prefix desktop run build`で実行します。RustのcargoがPATH上で使える環境では、`npm --prefix desktop run tauri -- dev`でTauriとViteを起動できます。Vite単独のプレビューにはTauriのネイティブ機能はありません。

Cargoの`custom-protocol`機能は配布時だけ有効にします。Tauri CLIのbuildとmacOSのビルドスクリプトがこれを設定し、開発時は`devUrl`のViteへ接続します。Cargoから直接配布用バイナリを作る場合は`--features custom-protocol`が必要です。

macOSのビルドは一時フォルダで署名検証まで済ませてから`dist/OpenRoom.app`を交換します。配置先のアプリや画像処理ワーカーが動いている場合は、上書きせずエラーで終了します。アプリを通常終了してから再実行してください。

macOSのビルドにはCommand Line Toolsと次の依存が必要です。

```sh
brew install libheif pkgconf cmake ninja gtkmm3 fftw libiptcdata lensfun exiv2 little-cms2 libomp librsvg fmt libraw
npm ci --prefix desktop
./scripts/build-desktop.sh
open dist/OpenRoom.app
```

初回ビルドでは`native/rtengine/source.json`で固定したソースを`.tools/rtengine/`へ取得し、パッチを適用してCMakeでビルドします。アプリにはエンジン、カメラデータ、プロファイル、ライセンス本文とパッチを同梱します。

`dist/OpenRoom.app`はローカル開発用のad-hoc署名です。公開配布用の署名・公証は行っていません。最低OSはビルドに使うlibheifなどの依存ライブラリにも依存します。

`./scripts/cargo-local.sh`はPATHのcargoを使用し、なければ一時領域のRustツールチェーンを探します。一時領域が消えた場合はRustを導入し直してください。

Windows/Linuxでも共通の検索・カタログ・描画処理を使用します。パッケージ作成には各OSのTauriビルド環境とlibheifが必要です。Linuxはlibheif-devとHEVC/AV1デコーダー、Windowsはvcpkgのlibheifを使用します。他OSでの起動は今回未確認です。

## データと検証

アプリの識別子は既存の`local.advanced-explorer.desktop`を維持します。macOSの標準カタログは`~/Library/Application Support/local.advanced-explorer.desktop/catalog.db`、プレビューはカタログに隣接する`catalog.db.previews/`、現像用作業キャッシュはその中の`working/`です。テスト用カタログは`AE_DATA_DIR`で分離できます。

隠しファイル、シンボリックリンク、アプリや写真ライブラリの内部は走査しません。アクセスできないルートを空として扱わず、タグやメモを保護します。カタログ保守用CLIにはバックアップと整合性検査を残しています。

```sh
npm ci --prefix desktop
npm --prefix desktop run build
npm --prefix desktop run test:unit
./scripts/cargo-local.sh test --locked
./scripts/cargo-local.sh test --manifest-path crates/media-core/Cargo.toml --locked
./scripts/cargo-local.sh test --manifest-path crates/develop-core/Cargo.toml --locked
./scripts/cargo-local.sh test --manifest-path desktop/Cargo.toml --locked
./scripts/cargo-local.sh clippy --locked --all-targets -- -D warnings
./scripts/cargo-local.sh clippy --manifest-path crates/media-core/Cargo.toml --locked --all-targets -- -D warnings
./scripts/cargo-local.sh clippy --manifest-path crates/develop-core/Cargo.toml --locked --all-targets -- -D warnings
./scripts/cargo-local.sh clippy --manifest-path desktop/Cargo.toml --locked --all-targets -- -D warnings
node scripts/test-desktop-ui.cjs
```

画面テストはTauriが同梱する`desktop/ui-dist/`を使います。Playwrightはnpm依存に含まれ、Chromiumは`desktop/`で`npx playwright install chromium`を実行して導入します。独自のChromium実行ファイルは`AE_CHROMIUM_EXECUTABLE`で指定できます。`npm --prefix desktop run test:ui`は単体テストとフロントのビルドを済ませてから画面テストを実行します。画面テストはTauri呼び出しを模擬するため、デコーダーやネイティブアプリの動作確認とは別です。実RAW/HEIFサンプルを使うテストは`AE_RAW_ASSETS`を指定して実行します。

依存ライブラリの出典と配布時の注意は[THIRD_PARTY.md](THIRD_PARTY.md)を参照してください。

## 配布前の確認

RAW検証用の写真はリポジトリに同梱せず、`reports/raw-heif-samples.json`に固定した公開サンプルを必要時に取得します。同梱テスト画像の出典は[fixtureの説明](crates/media-core/tests/fixtures/README.md)に記載しています。

共有前に`python3 scripts/test-privacy-check.py`と`python3 scripts/privacy-check.py --tracked`を実行してください。配布アプリは`python3 scripts/privacy-check.py dist/OpenRoom.app`でも確認できます。この検査はローカルパスや既知の秘密情報の形式を検出しますが、画像内の個人情報などを自動判定するものではありません。新しい画像やアーカイブを追加する場合は内容とメタデータも確認してください。

開発フォルダ全体をZIPにすると、Gitの除外設定に関係なくローカルデータが入ります。ソースの配布には監査済みコミットの`git archive`、アプリの配布には`dist/OpenRoom.app`だけを使用してください。アプリを起動すると作成される利用者のカタログとキャッシュは、配布ファイルとは別に保存されます。
