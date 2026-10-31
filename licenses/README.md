# 第三者ライセンス

このフォルダには、macOS版OpenRoomの依存パッケージが提供するライセンス本文と著作権表示を収録しています。本文は原文のまま保持しています。各パッケージの名称、版、出典、収録ファイルは[manifest.json](manifest.json)に記載しています。

一覧の対象は、macOS arm64向けのCargo依存関係、フロントエンドの実行時依存、配布アプリに実際に同梱したネイティブライブラリです。Cargoの一覧にはビルド用・テスト用の依存も含まれます。Windows・Linuxの配布物を網羅した一覧ではありません。

`rust/`にはCargoパッケージの表示を、`npm/`にはReact・React DOM・Schedulerの表示を、`native/`には同梱ライブラリとRawTherapee内の第三者コードの表示を収録しています。独自に修正したRawlerとRawTherapeeの差分は、ソースの`native/`にあります。Colourのデータに付随する著作権表示は[THIRD_PARTY.md](../THIRD_PARTY.md)に保持しています。

ネイティブライブラリの版は、配布アプリとインストール済みライブラリのMach-O UUIDを照合して特定しました。Homebrewのビルド定義も収録しています。GLibとLensfunの本文は、ビルド定義に記録されたSHA-256と一致するソースアーカイブから取得しました。

同梱DCP・ICC・ICMプロファイル218件とカメラデータ2件のSHA-256と埋め込み著作権表示は[profile-notices.json](profile-notices.json)に記録しています。

`libintl`のLGPL-2.1本文、OpenMPコンポーネントのApache-2.0＋LLVM例外の本文、Homebrewの本文とGLibのビルド用パッチを追加しました。`libomp`は元のHomebrew定義にMITと書かれていますが、上流の本文に従いApache-2.0 WITH LLVM-exceptionとして記録しています。

固定版の依存ソース606アーカイブの取得元とハッシュは[source-archives.json](source-archives.json)に記録しています。アーカイブ本体は[v0.7.9のリリース](https://github.com/wandering-beans/OpenRoom/releases/tag/v0.7.9)から取得できます。

`simd_helpers 0.1.0`のMITライセンス本文を収録しています。上流の[ライセンス追加コミット](https://github.com/lu-zero/simd_helpers/commit/82040194cd05affb060bf94d6f19f82a771d07fb)は、パッケージの固定コミットの直後に本文だけを追加しており、コードに変更がないことを確認しました。

依存関係の更新時は、一覧と本文も更新する必要があります。
