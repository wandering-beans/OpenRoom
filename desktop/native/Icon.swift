import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
let c=CGContext(data:nil,width:512,height:512,bitsPerComponent:8,bytesPerRow:0,space:CGColorSpace(name:CGColorSpace.sRGB)!,bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue)!
c.setFillColor(CGColor(red:0.2,green:0.3,blue:0.24,alpha:1));c.addPath(CGPath(roundedRect:CGRect(x:20,y:20,width:472,height:472),cornerWidth:105,cornerHeight:105,transform:nil));c.fillPath();c.setStrokeColor(CGColor(red:0.94,green:0.94,blue:0.86,alpha:1));c.setLineWidth(35);c.setLineCap(.round);c.move(to:CGPoint(x:130,y:135));c.addLine(to:CGPoint(x:245,y:375));c.addLine(to:CGPoint(x:355,y:135));c.move(to:CGPoint(x:175,y:230));c.addLine(to:CGPoint(x:312,y:230));c.strokePath();c.setStrokeColor(CGColor(red:0.89,green:0.63,blue:0.38,alpha:1));c.setLineWidth(22);c.move(to:CGPoint(x:335,y:335));c.addLine(to:CGPoint(x:400,y:400));c.addLine(to:CGPoint(x:346,y:400));c.move(to:CGPoint(x:400,y:400));c.addLine(to:CGPoint(x:400,y:346));c.strokePath()
let dest=CGImageDestinationCreateWithURL(URL(fileURLWithPath:CommandLine.arguments[1]) as CFURL,UTType.png.identifier as CFString,1,nil)!;CGImageDestinationAddImage(dest,c.makeImage()!,nil);if !CGImageDestinationFinalize(dest){exit(1)}
