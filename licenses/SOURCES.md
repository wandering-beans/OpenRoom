# 配布アプリに対応するソース

OpenRoom 0.7.9のmacOS arm64版に対応するソースを収集しました。[source-archives.json](source-archives.json)に取得元、固定版またはコミット、照合したハッシュ、アーカイブ名を記録しています。アーカイブ本体はローカルに保管しており、まだリリースへ添付していません。

| 対象 | アーカイブ数 | 照合方法 |
| --- | ---: | --- |
| Cargoパッケージ | 494 | 各Cargo.lockのSHA-256 |
| ネイティブ依存 | 26 | 24件はHomebrew定義のSHA-256、AOMは固定コミット、RawTherapeeは固定コミットのGitアーカイブ |
| GLibのビルド用依存 | 1 | gobject-introspectionのHomebrew定義のSHA-256 |
| npmパッケージ | 83 | package-lock.jsonのintegrity。ビルド用・他OS向けの任意依存も含む |
| React・Schedulerの元ソース | 2 | npmの各公開版が示すgitHead。ReactとReact DOMは同じコミット |
| 合計 | 606 | |

独自修正したRawlerはアーカイブ数に含めず、OpenRoom本体の`native/rawler/`から提供します。npmパッケージだけでは元の開発用ソースを含まない場合があるため、実行時依存のReact・React DOM・Schedulerについては上流の元ソースも収集しました。

## 差分とビルド定義

RawTherapeeのソース差分は`native/rtengine/float-preview.patch`と一致することを確認しました。OpenRoomのC ABIブリッジ、Rust側のコード、ビルド・インストールスクリプトは本体ソースに含まれます。

ネイティブ依存のHomebrew定義は各`native/<名前>-<版>/build-formula.rb`にあります。GLibの定義から参照される[パッチ](native/glib-2.90.1/Patches/glib/hardcoded-paths.diff)も収録しました。固定版ソースへの適用確認が通っています。パッチの最終変更は[2025年10月15日のコミット](https://github.com/Homebrew/homebrew-core/commit/1cf441a09f572566b1b7c4695a4d169d1177248a)で、同梱ライブラリの元ソース時刻より前でした。上流の定義は、bottleブロックを除いてインストール済み定義と一致します。

Homebrew定義内の`inreplace`による変更も再ビルド時に必要です。元ソースだけをビルドして、同じ設定・差分を再現したとは扱いません。Homebrewの定義とパッチには[Homebrewのライセンス](native/Homebrew-core/LICENSE.txt)も保持します。

## 再ビルド・差し替え

本体ソースと依存ソースを取得し、[開発ガイド](../DEVELOPMENT.md)の手順に沿ってビルドします。依存の版、各定義のオプション、差分、アーキテクチャを揃える必要があります。Cargo・npmのアーカイブは公開時の配布物をそのまま保存しており、展開・キャッシュ設定を済ませたオフライン環境ではまだビルドしていません。

LGPLの動的ライブラリは`OpenRoom.app/Contents/Frameworks`にあります。同じABIを保った修正版に差し替える場合は、変更したアプリに`codesign --force --deep --sign - OpenRoom.app`でローカル署名を付け直す方法があります。実アプリを使った修正版ライブラリへの差し替え確認は未実施です。自分でビルドして使うために、OpenRoomの更新配信用秘密鍵を取得する必要はありません。

## 公開前に残る作業

- [プロファイルとカメラデータ](PROFILE-LICENSES.md)の未確認条件を解消する。
- 収集したソース、差分、ビルド定義からの再ビルド手順と、LGPLライブラリの差し替えを確認する。
- 今回補ったライセンス本文を同梱したアプリを用意し、対応ソース一式を同じ受領者が取得できる場所へ配置する。

ファイルの取得とハッシュ一致は確認済みですが、これだけで対応ソースの提供完了や公開条件の充足を宣言するものではありません。[GNUの説明](https://www.gnu.org/licenses/gpl-faq.en.html#UnchangedJustBinary)に沿って、バイナリに対応するソースと改変箇所を提供します。
