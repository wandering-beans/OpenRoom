# OpenRoomのアプリアイコン

`icon.png`・`icon.icns`・`icon.ico`は採用ロゴの書き出しです。

Mac版では、`AppIcon.icon`からXcodeの`actool`でコンパイルした`Assets.car`も同梱します。`CFBundleIconName`を`AppIcon`に設定し、macOS 27で従来形式のアイコンに付いていた白い外枠を解消しています。`icon.icns`は従来のmacOS向けに残します。

`AppIcon.icon/Assets/OpenRoom.png`は`icon.png`と同じ画像です。パッケージ側で不透明な黒背景を指定し、反射、影、透過の装飾を無効にしています。文字と分光の画像自体は変更していません。

## 再生成

リポジトリのルートで実行します。コンパイルにはXcodeが必要ですが、生成済みの`Assets.car`を使う通常のビルドでは不要です。

```sh
mkdir -p /private/tmp/openroom-icon-build
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun actool \
  desktop/icons/AppIcon.icon \
  --compile /private/tmp/openroom-icon-build \
  --platform macosx --minimum-deployment-target 14.0 \
  --app-icon AppIcon \
  --output-partial-info-plist /private/tmp/openroom-icon-build/icon-info.plist
cp /private/tmp/openroom-icon-build/Assets.car desktop/icons/Assets.car
```

通常のTauriバンドルと`scripts/build-desktop.sh`の両方が、`Assets.car`と`CFBundleIconName`を設定します。

[Appleのアイコン設定資料](https://developer.apple.com/documentation/xcode/configuring-your-app-icon)と[Icon Composer形式の説明](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer)を参照できます。
