# 第三者ライセンス

このフォルダには、macOS版OpenRoomの依存パッケージが提供するライセンス本文と著作権表示を収録しています。本文は原文のまま保持しています。各パッケージの名称、版、出典、収録ファイルは[manifest.json](manifest.json)に記載しています。

一覧の対象は、macOS arm64向けのCargo依存関係、フロントエンドの実行時依存、配布アプリに実際に同梱したネイティブライブラリです。Cargoの一覧にはビルド用・テスト用の依存も含まれます。Windows・Linuxの配布物を網羅した一覧ではありません。

`rust/`にはCargoパッケージの表示を、`npm/`にはReact・React DOM・Schedulerの表示を、`native/`には同梱ライブラリとRawTherapee内の第三者コードの表示を収録しています。独自に修正したRawlerとRawTherapeeの差分は、ソースの`native/`にあります。Colourのデータに付随する著作権表示は[THIRD_PARTY.md](../THIRD_PARTY.md)に保持しています。

ネイティブライブラリの版は、配布アプリとインストール済みライブラリのMach-O UUIDを照合して特定しました。Homebrewのビルド定義も収録しています。GLibとLensfunの本文は、ビルド定義に記録されたSHA-256と一致するソースアーカイブから取得しました。

同梱DCP・ICC・ICMプロファイル218件とカメラデータ2件を[profile-notices.json](profile-notices.json)に記録しています。実ファイルのSHA-256、埋め込み著作権表示、根拠の区分を保持し、[適用条件の確認状況](PROFILE-LICENSES.md)を別途記載しています。

`libintl`のLGPL-2.1本文、OpenMPコンポーネントのApache-2.0＋LLVM例外の本文、Homebrewの本文とGLibのビルド用パッチを追加しました。`libomp`は元のHomebrew定義にMITと書かれていますが、上流の本文に従いApache-2.0 WITH LLVM-exceptionとして記録しています。

固定版の依存ソース606アーカイブを取得・照合しました。取得元とハッシュは[source-archives.json](source-archives.json)、収録範囲と再ビルドに残る作業は[SOURCES.md](SOURCES.md)を参照してください。

`simd_helpers 0.1.0`のMITライセンス本文を収録しています。上流の[ライセンス追加コミット](https://github.com/lu-zero/simd_helpers/commit/82040194cd05affb060bf94d6f19f82a771d07fb)は、パッケージの固定コミットの直後に本文だけを追加しており、コードに変更がないことを確認しました。

この一覧は、依存ソース一式や個別のライセンス適合性の確認に代わるものではありません。配布前に[配布手順](../DISTRIBUTION.md)を確認してください。依存関係の更新時は、一覧と本文も更新する必要があります。
