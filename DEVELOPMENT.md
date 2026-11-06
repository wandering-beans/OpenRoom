# 開発ガイド

[利用ガイドに戻る](README.md)

## ビルドと起動
Rust stable、Node.js 22.12以上と対象OSのビルド環境が必要です。デスクトップはTauri 2＋React＋TypeScript＋Viteです。ライブラリの未編集プレビューはmacOSでSwiftとImageIO/Core Imageを使用します。macOSとWindows x64の現像はrtengineとRustの組み合わせで、RAWには同じコミットとパッチで固定したrtengineを使います。Linuxと`--no-default-features`では従来のRAW現像を使います。rawlerは埋め込みプレビューやメタデータの読み込みにも必要なため残しています。HEIF/AVIFは全OSでlibheifを使います。

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

Windows/Linuxでも共通の検索・カタログ・描画処理を使用します。Linuxはlibheif-devとHEVC/AV1デコーダー、Windowsはvcpkgのlibheifを使用します。

### Windows x64のコア開発

Windows 11 x64ではRustのMSVCツールチェーン、Visual Studio 2022 Build ToolsのC++開発ツールとWindows SDK、Node.js 24、MSYS2 UCRT64を使用します。Tauriの画面表示にはWebView2も必要です。rtengineだけをUCRT64でDLLにし、Rustとは既存のC ABIで接続します。C++オブジェクトの生成と解放はDLL内で完結します。

MSYS2のUCRT64ターミナルで次のパッケージを導入します。

```sh
pacman -Syu
pacman -S --needed mingw-w64-ucrt-x86_64-{gcc,cmake,ninja,pkgconf,gtkmm3,lcms2,fftw,lensfun,libiptcdata,exiv2,fmt,libraw,python}
```

PowerShellでvcpkgを用意し、HEVCとAV1に対応するlibheifをビルドします。すでにvcpkgがある場合は、その環境を使えます。

```powershell
git clone https://github.com/microsoft/vcpkg.git .tools/vcpkg
& .tools/vcpkg/bootstrap-vcpkg.bat -disableMetrics
& .tools/vcpkg/vcpkg.exe install 'libheif[aom]:x64-windows-static-md'
```

リポジトリ直下で次を実行します。`windows-dev.ps1`の既定のMSYS2配置先は`$env:USERPROFILE\.local\msys64\ucrt64`です。別の場所に導入した場合は`-Msys2Prefix`、vcpkgの配置先には`-VcpkgRoot`を指定します。PythonはUCRT64のものを使い、`-Python`で別の実行ファイルも指定できます。

```powershell
. ./scripts/windows-dev.ps1
cargo test --manifest-path crates/rtengine/Cargo.toml --locked
cargo test --manifest-path crates/develop-core/Cargo.toml --locked
& $env:OPENROOM_PYTHON scripts/test-rtengine-contract.py .tools/rtengine/build-windows-x86_64/openroom_rtengine.dll
npm ci --prefix desktop
npm --prefix desktop run build
cargo test --manifest-path desktop/Cargo.toml --locked
cargo build --manifest-path desktop/Cargo.toml --locked
```

`OPENROOM_MSYS2_PREFIX`と`OPENROOM_PYTHON`はrtengineのビルド専用です。GCCの実行ファイルを含むディレクトリをRustビルド全体のPATHに追加する必要はありません。DLLとその依存DLLはCargoの出力先と`deps/`に配置されます。開発時のカメラデータは`.tools/rtengine/data/`を使います。配布用のデータ探索先は、Windowsでは実行ファイルに隣接する`rtengine/`、macOSでは`Contents/Resources/rtengine/`です。Windowsインストーラーは、以下の手順で作成できます。

Windowsでも通常プレビューと原寸処理に別々の常駐ワーカーを使い、現像セッションと調整結果のキャッシュを再利用します。Windowsのパイプ書き込みは専用スレッドで処理し、画像バッファを追加コピーせずに渡します。キャンセルやタイムアウトでは子プロセスを終了し、次のリクエストで再作成します。macOSの通信方式、Metal処理、現像アルゴリズムは維持しています。

CIにはmacOSとWindowsのコア検証を含めています。実機間の画素比較や速度比較には同じ写真・設定を使い、LibRawなどの依存バージョンも記録してください。同じrtengineでも、CPUやコンパイラによる浮動小数点演算の差があるため、OS間の完全な画素一致は保証していません。Windows ARM64とLinuxのrtengine対応は今回の対象外です。

### Windowsインストーラー

`scripts/windows-dev.ps1`を読み込んだPowerShellで、`& $env:OPENROOM_PYTHON scripts/build-windows.py`を実行します。Node.jsもPATHに必要です。出力は`dist/OpenRoom-VERSION-windows-x64-setup.exe`とSHA-256ファイルです。rtengineのDLL、依存DLL、カメラデータとライセンスを同梱します。現在のユーザーの`%LOCALAPPDATA%/OpenRoom`にインストールし、スタートメニューに登録します。管理者権限は不要です。

### WindowsのGPU処理

Windowsでは既定でDirect3D 11のコンピュートシェーダーを使い、対応する高性能GPUを優先します。露出、ホワイトバランス、トーン、カーブ、彩度、カラーミキサー、モノクロ、カラーグレーディング、周辺減光、粒子の全体補正が対象です。macOSでは既存のMetal処理を使います。両OSで補正パラメーターの組み立てを共有し、WindowsのHDR変換にもCPUで生成したテーブルを使います。

512×512画素未満の画像と旧RAW色変換はCPUで処理します。GPUの初期化、メモリ確保、実行に失敗した場合も、入力画像を変更せずCPU処理へ戻ります。GPUの応答待ちは5秒までとし、応答停止やデバイス喪失後はワーカーを再起動するまでCPUを使います。表示用のGPUバッファは再利用し、原寸画像の大きなバッファは処理後に解放します。

RAWのデコード、rtengineのノイズ除去・シャープ処理、幾何変換、ローカルマスク、ファイルへの書き出しは引き続きCPUで処理します。特に書き出しは既存の16 bit精度を維持するため、GPUへ切り替えていません。

GPUを無効にして比較する場合は、PowerShellで`$env:OPENROOM_DISABLE_GPU = '1'`を設定してからアプリを起動します。設定を外すには`Remove-Item Env:OPENROOM_DISABLE_GPU`を実行し、アプリを再起動してください。

実GPUがある環境では、次のテストでCPUとの画素差、透過度、原寸画像の処理、GPU無効時の動作を確認できます。速度は最適化ビルドで測り、GPUへの転送と読み戻しを含めます。CIではGPU実機テストを自動実行せず、HLSLのコンパイルとCPUへの切り替えを検証します。

```powershell
. ./scripts/windows-dev.ps1
cargo test --manifest-path crates/develop-core/Cargo.toml --locked --release --lib color::gpu::tests -- --include-ignored --nocapture --test-threads=1
```

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


## 署名付きアプリ更新

更新処理はRustの`desktop/src/updates.rs`で行い、画面は`UpdateSettings.tsx`で管理します。配布先はOpenRoomの固定GitHub URLに限定し、TLS、更新用署名、署名に含まれる版番号とファイル名を検証します。OS・CPUごとの対象を明示し、WindowsではPE実行ファイル、macOSではgzipアーカイブであることも確認します。対象の配布がなければエラーを表示し、別のOS用ファイルは選びません。更新ファイルはカタログや写真を含みません。取得・署名検証が失敗した場合はインストールせず、取得したデータは破棄します。

公開リポジトリの更新情報と添付ファイルを認証なしで取得します。更新情報を取得できない場合はエラーを表示し、「最新」とは表示しません。GitHubのトークンをアプリへ埋め込まないでください。

macOSの更新は、次の順で準備します。

1. `desktop/Cargo.toml`、`desktop/tauri.conf.json`、`desktop/package.json`と各lockファイルのバージョンをそろえ、`bundle.macOS.bundleVersion`を前回より増やす。依存・版の変更後はライセンス目録も更新する。
2. `sh scripts/build-desktop.sh dist/update-preview/OpenRoom.app`で配布アプリを別の場所にビルドする。引数なしの場合は従来どおり`dist/OpenRoom.app`へ配置する。
3. `TAURI_SIGNING_PRIVATE_KEY_PATH`に秘密鍵ファイルの場所、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`に鍵のパスワードを設定する。秘密鍵と同じ場所の`.pub`ファイルをアプリ設定内の公開鍵と照合する。秘密鍵はGit・ソースZIP・アプリに含めず、紛失に備えて別の安全な場所へバックアップする。
4. `python3 scripts/package-update.py dist/update-preview/OpenRoom.app --notes release-notes.txt --output dist/releases/vVERSION-update`を実行する。`VERSION`は設定した版番号に置き換える。出力済みのフォルダは上書きしない。
5. GitHubの同じ版のリリースへ`.app.tar.gz`、`.sig`、チェックサムと対応するソースを添付する。配布条件とダウンロードを確認してから、生成された`latest.json`を`updates/latest.json`へコピーしてコミットする。更新情報をアプリより先に公開しない。

Windowsでは`build-windows.py`の出力を入力にして、`python scripts/package-update.py dist/OpenRoom-VERSION-windows-x64-setup.exe --notes release-notes.txt --output dist/releases/vVERSION-windows-update`を実行します。署名鍵と版番号の管理はmacOSと共通です。生成した`.exe`、`.sig`、チェックサムと対応するソースをGitHubの同じバージョンのリリースへ添付してから、`windows-x86_64.json`を`updates/windows-x86_64.json`へコピーします。macOS用の`updates/latest.json`は置き換えません。初期のWindows更新情報は配布対象が空で、署名済みのWindows更新を公開するまでは「更新はまだ公開されていません」と表示されます。

Windowsのアプリ内更新はNSISでインストールしたアプリから実行します。署名と形式の検証後にアプリを終了し、インストーラーが置き換えと再起動を行います。開発用の実行ファイルからは更新できません。

Tauriの更新用署名と、WindowsのAuthenticode署名、macOSのDeveloper ID署名・公証は別です。WindowsインストーラーにAuthenticode署名は付けていません。現在のmacOSアプリはアドホック署名で、Developer ID署名・公証は行っていません。最初のインストール時のmacOSの確認は、更新用署名では省略されません。

更新機能に限定したテストは、フロントのビルド後に`node scripts/test-desktop-ui.cjs --updates`、Rustは`cargo test --manifest-path desktop/Cargo.toml updates::tests --locked`で実行できます。後者は一時的なローカルサーバーと使い捨ての署名鍵で、OSの選択・改ざん拒否・版番号とファイル名の照合を確認します。一時アプリの置き換えテストはmacOSで実行します。Windows用の署名作成と配布先の分離は`python scripts/test-package-update.py`で検証できます。

配布ファイル自体を検証する場合は、`OPENROOM_UPDATE_PACKAGE`にパッケージ作成先を指定し、`cargo test --manifest-path desktop/Cargo.toml updates::tests::packaged_release --locked -- --ignored`を実行します。本番公開鍵で署名を確認して一時フォルダへインストールし、アプリのmacOS署名・バージョン・カタログ保持を確認します。
