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

let background = rgb(0xF8, 0xFB, 0xFF)
let panel = rgb(0xFF, 0xFF, 0xFF)
let primary = rgb(0x25, 0x63, 0xEB)
let primaryLight = rgb(0x1D, 0x4E, 0xD8)
let foreground = rgb(0x03, 0x07, 0x12)
let mutedForeground = rgb(0x5B, 0x65, 0x75)

ctx.setFillColor(background)
ctx.fill(CGRect(x: 0, y: 0, width: widthPt, height: heightPt))

func radialBlob(center: CGPoint, radius: CGFloat, color: CGColor, alpha: CGFloat) {
    let comps = color.components ?? [0, 0, 0, 1]
    let inner = CGColor(srgbRed: comps[0], green: comps[1], blue: comps[2], alpha: alpha)
    let outer = CGColor(srgbRed: comps[0], green: comps[1], blue: comps[2], alpha: 0)
    let gradient = CGGradient(
        colorsSpace: colorSpace,
        colors: [inner, outer] as CFArray,
        locations: [0, 1]
    )!
    ctx.drawRadialGradient(
        gradient,
        startCenter: center,
        startRadius: 0,
        endCenter: center,
        endRadius: radius,
        options: []
    )
}

radialBlob(center: CGPoint(x: 400, y: 270), radius: 280, color: primary, alpha: 0.16)
radialBlob(center: CGPoint(x: 150, y: 350), radius: 260, color: primary, alpha: 0.10)
radialBlob(center: CGPoint(x: 270, y: 120), radius: 230, color: primary, alpha: 0.06)

ctx.saveGState()
ctx.setStrokeColor(CGColor(srgbRed: 0.37, green: 0.48, blue: 0.68, alpha: 0.12))
ctx.setLineWidth(0.7)
let gridSpacing: CGFloat = 32
var gridX: CGFloat = 0
while gridX <= widthPt {
    ctx.move(to: CGPoint(x: gridX, y: 0))
    ctx.addLine(to: CGPoint(x: gridX, y: heightPt))
    gridX += gridSpacing
}
var gridY: CGFloat = 0
while gridY <= heightPt {
    ctx.move(to: CGPoint(x: 0, y: gridY))
    ctx.addLine(to: CGPoint(x: widthPt, y: gridY))
    gridY += gridSpacing
}
ctx.strokePath()
ctx.restoreGState()

func roundedRect(_ rect: CGRect, radius: CGFloat) -> CGPath {
    CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)
}

func drawCard(center: CGPoint) {
    let rect = CGRect(x: center.x - 76, y: center.y - 74, width: 152, height: 148)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -14), blur: 28, color: CGColor(srgbRed: 0.10, green: 0.20, blue: 0.40, alpha: 0.14))
    ctx.setFillColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.68))
    ctx.addPath(roundedRect(rect, radius: 24))
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.setStrokeColor(CGColor(srgbRed: 0.18, green: 0.38, blue: 0.92, alpha: 0.14))
    ctx.setLineWidth(1)
    ctx.addPath(roundedRect(rect, radius: 24))
    ctx.strokePath()
    ctx.restoreGState()
}

let iconLeftCenter = CGPoint(x: 150, y: heightPt - 170)
let iconRightCenter = CGPoint(x: 390, y: heightPt - 170)
drawCard(center: iconLeftCenter)
drawCard(center: iconRightCenter)

let arrowY = iconLeftCenter.y + 6
let arrowStartX = iconLeftCenter.x + 88
let arrowEndX = iconRightCenter.x - 88

ctx.saveGState()
ctx.setStrokeColor(primary)
ctx.setLineWidth(2.8)
ctx.setLineCap(.round)
ctx.setLineJoin(.round)
ctx.move(to: CGPoint(x: arrowStartX, y: arrowY))
ctx.addLine(to: CGPoint(x: arrowEndX, y: arrowY))
ctx.strokePath()

let head: CGFloat = 10
ctx.move(to: CGPoint(x: arrowEndX, y: arrowY))
ctx.addLine(to: CGPoint(x: arrowEndX - head, y: arrowY + head * 0.62))
ctx.move(to: CGPoint(x: arrowEndX, y: arrowY))
ctx.addLine(to: CGPoint(x: arrowEndX - head, y: arrowY - head * 0.62))
ctx.strokePath()
ctx.restoreGState()

func drawText(_ string: String, at point: CGPoint, font: NSFont, color: CGColor, centered: Bool = true, kern: CGFloat = 0.2) {
    let nsColor = NSColor(cgColor: color) ?? NSColor.white
    let paragraph = NSMutableParagraphStyle()
    paragraph.alignment = centered ? .center : .left
    let attrs: [NSAttributedString.Key: Any] = [
        .font: font,
        .foregroundColor: nsColor,
        .paragraphStyle: paragraph,
        .kern: kern,
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
        if let font = NSFont(name: name, size: size) {
            return font
        }
    }
    return NSFont.systemFont(ofSize: size, weight: weight)
}

let badgeFont = bestFont(["Poppins-SemiBold", "Poppins"], size: 10, weight: .semibold)
let titleFont = bestFont(["Poppins-SemiBold", "Poppins-Medium", "Poppins"], size: 17, weight: .semibold)
let captionFont = bestFont(["Poppins-Regular", "Poppins"], size: 11.5, weight: .regular)

let badgeRect = CGRect(x: 166, y: 124, width: 208, height: 27)
ctx.saveGState()
ctx.setFillColor(CGColor(srgbRed: 0.145, green: 0.388, blue: 0.922, alpha: 0.16))
ctx.addPath(roundedRect(badgeRect, radius: 13.5))
ctx.fillPath()
ctx.setStrokeColor(CGColor(srgbRed: 0.36, green: 0.58, blue: 0.98, alpha: 0.36))
ctx.setLineWidth(1)
ctx.addPath(roundedRect(badgeRect, radius: 13.5))
ctx.strokePath()
ctx.restoreGState()

drawText("AGENT REVIEW WORKSPACE", at: CGPoint(x: badgeRect.midX, y: badgeRect.midY - 0.5), font: badgeFont, color: primaryLight, kern: 1.2)
drawText("Drag Reviu to Applications", at: CGPoint(x: widthPt / 2, y: 92), font: titleFont, color: foreground)
drawText("The review app for code your agent writes", at: CGPoint(x: widthPt / 2, y: 66), font: captionFont, color: mutedForeground)

ctx.saveGState()
ctx.setFillColor(panel.copy(alpha: 0.28) ?? panel)
ctx.fill(CGRect(x: 0, y: 0, width: widthPt, height: 22))
ctx.restoreGState()

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
    kCGImagePropertyDPIHeight: 144,
]
CGImageDestinationAddImage(dest, cgImage, props as CFDictionary)
guard CGImageDestinationFinalize(dest) else {
    fatalError("Failed to write PNG")
}

print("Wrote \(outURL.path) (\(widthPx)x\(heightPx))")
