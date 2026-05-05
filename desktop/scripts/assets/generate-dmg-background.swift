import Foundation
import AppKit
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

let scale: CGFloat = 2
let widthPt: CGFloat = 540
let heightPt: CGFloat = 460
let widthPx = Int(widthPt * scale)
let heightPx = Int(heightPt * scale)

let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
guard let ctx = CGContext(
    data: nil,
    width: widthPx,
    height: heightPx,
    bitsPerComponent: 8,
    bytesPerRow: 0,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    fatalError("Failed to create context")
}

ctx.scaleBy(x: scale, y: scale)

func rgb(_ r: Int, _ g: Int, _ b: Int, _ a: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat(r) / 255, green: CGFloat(g) / 255, blue: CGFloat(b) / 255, alpha: a)
}

let white = rgb(255, 255, 255)
let lightBlue = rgb(0x80, 0xCA, 0xFF)
let indigo = rgb(0x4F, 0x46, 0xE5)
let primary = rgb(0x25, 0x63, 0xEB)
let mutedFg = rgb(120, 120, 130)

ctx.setFillColor(white)
ctx.fill(CGRect(x: 0, y: 0, width: widthPt, height: heightPt))

func radialBlob(center: CGPoint, radius: CGFloat, color: CGColor, alpha: CGFloat) {
    let comps = color.components ?? [0, 0, 0, 1]
    let r = comps[0], g = comps[1], b = comps[2]
    let inner = CGColor(srgbRed: r, green: g, blue: b, alpha: alpha)
    let outer = CGColor(srgbRed: r, green: g, blue: b, alpha: 0)
    let gradient = CGGradient(
        colorsSpace: colorSpace,
        colors: [inner, outer] as CFArray,
        locations: [0, 1]
    )!
    ctx.saveGState()
    ctx.drawRadialGradient(
        gradient,
        startCenter: center,
        startRadius: 0,
        endCenter: center,
        endRadius: radius,
        options: []
    )
    ctx.restoreGState()
}

radialBlob(center: CGPoint(x: 80, y: 400), radius: 460, color: primary, alpha: 0.28)
radialBlob(center: CGPoint(x: 460, y: 100), radius: 460, color: primary, alpha: 0.20)
radialBlob(center: CGPoint(x: 270, y: 230), radius: 280, color: primary, alpha: 0.08)

ctx.saveGState()
ctx.setFillColor(CGColor(srgbRed: 0.15, green: 0.18, blue: 0.30, alpha: 0.06))
let dotSpacing: CGFloat = 24
let dotRadius: CGFloat = 1.0
var y: CGFloat = dotSpacing / 2
while y < heightPt {
    var x: CGFloat = dotSpacing / 2
    while x < widthPt {
        ctx.fillEllipse(in: CGRect(x: x - dotRadius, y: y - dotRadius, width: dotRadius * 2, height: dotRadius * 2))
        x += dotSpacing
    }
    y += dotSpacing
}
ctx.restoreGState()

let iconLeftCenter = CGPoint(x: 150, y: heightPt - 170)
let iconRightCenter = CGPoint(x: 390, y: heightPt - 170)
let arrowY = iconLeftCenter.y
let arrowStartX = iconLeftCenter.x + 76
let arrowEndX = iconRightCenter.x - 76

ctx.saveGState()
ctx.setStrokeColor(primary)
ctx.setLineWidth(2.5)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)
ctx.move(to: CGPoint(x: arrowStartX, y: arrowY))
ctx.addLine(to: CGPoint(x: arrowEndX, y: arrowY))
ctx.strokePath()

let head: CGFloat = 10
ctx.move(to: CGPoint(x: arrowEndX, y: arrowY))
ctx.addLine(to: CGPoint(x: arrowEndX - head, y: arrowY + head * 0.6))
ctx.move(to: CGPoint(x: arrowEndX, y: arrowY))
ctx.addLine(to: CGPoint(x: arrowEndX - head, y: arrowY - head * 0.6))
ctx.strokePath()
ctx.restoreGState()

func drawText(_ string: String, at point: CGPoint, font: NSFont, color: CGColor, centered: Bool = true) {
    let nsColor = NSColor(cgColor: color) ?? NSColor.black
    let paragraph = NSMutableParagraphStyle()
    paragraph.alignment = centered ? .center : .left
    let attrs: [NSAttributedString.Key: Any] = [
        .font: font,
        .foregroundColor: nsColor,
        .paragraphStyle: paragraph,
        .kern: 0.2
    ]
    let attr = NSAttributedString(string: string, attributes: attrs)
    let size = attr.size()
    let origin = centered
        ? CGPoint(x: point.x - size.width / 2, y: point.y - size.height / 2)
        : point

    NSGraphicsContext.saveGraphicsState()
    let nsCtx = NSGraphicsContext(cgContext: ctx, flipped: false)
    NSGraphicsContext.current = nsCtx
    attr.draw(at: origin)
    NSGraphicsContext.restoreGraphicsState()
}

func bestFont(_ candidates: [String], size: CGFloat, weight: NSFont.Weight) -> NSFont {
    for name in candidates {
        if let f = NSFont(name: name, size: size) {
            return f
        }
    }
    return NSFont.systemFont(ofSize: size, weight: weight)
}

let titleFont = bestFont(["Poppins-SemiBold", "Poppins-Medium", "Poppins"], size: 16, weight: .semibold)
let captionFont = bestFont(["Poppins-Regular", "Poppins"], size: 11, weight: .regular)

drawText("Drag Reviu to Applications", at: CGPoint(x: widthPt / 2, y: 130), font: titleFont, color: primary)
drawText("Reviu - keyboard-first Git client", at: CGPoint(x: widthPt / 2, y: 104), font: captionFont, color: mutedFg)

guard let cgImage = ctx.makeImage() else {
    fatalError("Failed to make image")
}

let outArg = CommandLine.arguments.dropFirst().first ?? "dmg-background.png"
let outURL = URL(fileURLWithPath: outArg)

guard let dest = CGImageDestinationCreateWithURL(outURL as CFURL, UTType.png.identifier as CFString, 1, nil) else {
    fatalError("Failed to create destination")
}

let props: [CFString: Any] = [
    kCGImagePropertyDPIWidth: 144,
    kCGImagePropertyDPIHeight: 144
]
CGImageDestinationAddImage(dest, cgImage, props as CFDictionary)
guard CGImageDestinationFinalize(dest) else {
    fatalError("Failed to write PNG")
}

print("Wrote \(outURL.path) (\(widthPx)x\(heightPx))")
