import Foundation
import ImageIO
import CoreGraphics
import CoreImage
import UniformTypeIdentifiers

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data(message.utf8)); exit(1)
}
let args = CommandLine.arguments
guard args.count == 4, let bound = Int(args[3]), bound > 0, bound <= 4096 else { fail("Invalid rendering arguments") }
let url = URL(fileURLWithPath: args[1])
var rendered: CGImage?
var width = 0
var height = 0
guard let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary) else { fail("画像を読み込めません。") }
if let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any] {
    width = props[kCGImagePropertyPixelWidth] as? Int ?? 0
    height = props[kCGImagePropertyPixelHeight] as? Int ?? 0
}
let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: bound >= 4096 || (bound > 384 && url.pathExtension.lowercased() != "dng"), kCGImageSourceCreateThumbnailFromImageIfAbsent: true, kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: bound, kCGImageSourceShouldCacheImmediately: true]
rendered = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
if rendered == nil && url.pathExtension.lowercased() == "dng" {
    if let raw = CIRAWFilter(imageURL: url) {
        raw.scaleFactor = min(1, Float(bound) / Float(max(raw.nativeSize.width, raw.nativeSize.height)))
        if let output = raw.outputImage {
            rendered = CIContext(options: [.cacheIntermediates: false]).createCGImage(output, from: output.extent)
        }
    }
}
guard let image = rendered else { fail("このファイルを表示できません。DNGはmacOS対応のRAW形式に限ります。") }
guard let destination = CGImageDestinationCreateWithURL(URL(fileURLWithPath: args[2]) as CFURL, UTType.png.identifier as CFString, 1, nil) else { fail("キャッシュを書き込めません。") }
CGImageDestinationAddImage(destination, image, nil)
guard CGImageDestinationFinalize(destination) else { fail("プレビュー保存に失敗しました。") }
let metadata: [String: Any] = ["width": width, "height": height, "render_width": image.width, "render_height": image.height]
print(String(data: try JSONSerialization.data(withJSONObject: metadata, options: [.sortedKeys]), encoding: .utf8)!)
