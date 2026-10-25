import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
let folder=URL(fileURLWithPath:CommandLine.arguments[1]);try FileManager.default.createDirectory(at:folder,withIntermediateDirectories:true)
func picture(_ n:Int)->CGImage {
 let ctx=CGContext(data:nil,width:1600,height:1100,bitsPerComponent:8,bytesPerRow:0,space:CGColorSpace(name:CGColorSpace.sRGB)!,bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue)!
 let colors:[[CGFloat]]=[[0.72,0.78,0.64,1],[0.84,0.69,0.47,1],[0.54,0.67,0.71,1],[0.85,0.57,0.43,1]]
 ctx.setFillColor(colors[n%4]);ctx.fill(CGRect(x:0,y:0,width:1600,height:1100));ctx.setFillColor(CGColor(gray:0.95,alpha:1));ctx.fillEllipse(in:CGRect(x:100+n*25,y:460,width:460,height:460));ctx.setFillColor(CGColor(red:0.19,green:0.29,blue:0.22,alpha:1));ctx.move(to:CGPoint(x:0,y:0));ctx.addLine(to:CGPoint(x:600,y:700));ctx.addLine(to:CGPoint(x:1050,y:0));ctx.closePath();ctx.fillPath();ctx.setFillColor(CGColor(red:0.36,green:0.43,blue:0.3,alpha:1));ctx.move(to:CGPoint(x:700,y:0));ctx.addLine(to:CGPoint(x:1250,y:800));ctx.addLine(to:CGPoint(x:1600,y:400));ctx.addLine(to:CGPoint(x:1600,y:0));ctx.closePath();ctx.fillPath();return ctx.makeImage()!
}
for n in 0..<12 {
 let ext=n%2==0 ? "png":"jpeg";let url=folder.appendingPathComponent(String(format:"風景-%02d.%@",n,ext));let dest=CGImageDestinationCreateWithURL(url as CFURL,(ext=="png" ? UTType.png.identifier:UTType.jpeg.identifier) as CFString,1,nil)!;CGImageDestinationAddImage(dest,picture(n),nil);if !CGImageDestinationFinalize(dest) { fatalError("Fixture generation failed") }
}
