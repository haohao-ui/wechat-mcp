// Renders the 微信客服助手 app icon. Usage: swift draw_icon.swift <out.png> <pixels>
import AppKit

let args = CommandLine.arguments
let out = URL(fileURLWithPath: args[1])
let px = CGFloat(Double(args[2]) ?? 1024)
let s = px / 1024.0
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(px), pixelsHigh: Int(px), bitsPerSample: 8,
                           samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                           bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let ctx = NSGraphicsContext.current!.cgContext
ctx.scaleBy(x: s, y: s)

// macOS icon grid: 824pt body centred in 1024 with a soft drop shadow.
let body = CGRect(x: 100, y: 100, width: 824, height: 824)
let bodyPath = NSBezierPath(roundedRect: body, xRadius: 185, yRadius: 185)
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -10), blur: 28, color: NSColor.black.withAlphaComponent(0.30).cgColor)
NSColor(srgbRed: 0.05, green: 0.42, blue: 0.40, alpha: 1).setFill()
bodyPath.fill()
ctx.restoreGState()
bodyPath.addClip()
NSGradient(colors: [NSColor(srgbRed: 0.04, green: 0.36, blue: 0.42, alpha: 1),
                    NSColor(srgbRed: 0.10, green: 0.68, blue: 0.47, alpha: 1)])!.draw(in: body, angle: 70)
// Subtle top sheen.
NSGradient(colors: [NSColor.white.withAlphaComponent(0.16), NSColor.white.withAlphaComponent(0)])!
    .draw(in: CGRect(x: 100, y: 560, width: 824, height: 364), angle: -90)

// Headset band arching over the bubble.
let band = NSBezierPath()
band.appendArc(withCenter: CGPoint(x: 512, y: 500), radius: 270, startAngle: 200, endAngle: -20, clockwise: true)
band.lineWidth = 46
band.lineCapStyle = .round
NSColor.white.withAlphaComponent(0.92).setStroke()
band.stroke()
// Ear cups.
for x in [218.0, 702.0] {
    let cup = NSBezierPath(roundedRect: CGRect(x: x, y: 330, width: 104, height: 170), xRadius: 46, yRadius: 46)
    NSColor.white.setFill(); cup.fill()
}
// Microphone boom from the left cup.
let boom = NSBezierPath()
boom.move(to: CGPoint(x: 270, y: 340))
boom.curve(to: CGPoint(x: 430, y: 236), controlPoint1: CGPoint(x: 268, y: 262), controlPoint2: CGPoint(x: 340, y: 236))
boom.lineWidth = 26; boom.lineCapStyle = .round
NSColor.white.setStroke(); boom.stroke()
NSBezierPath(ovalIn: CGRect(x: 418, y: 210, width: 58, height: 52)).fill()

// Speech bubble between the cups.
let bubble = NSBezierPath(roundedRect: CGRect(x: 352, y: 352, width: 320, height: 236), xRadius: 92, yRadius: 92)
let tail = NSBezierPath()
tail.move(to: CGPoint(x: 600, y: 370)); tail.line(to: CGPoint(x: 646, y: 300)); tail.line(to: CGPoint(x: 540, y: 360)); tail.close()
bubble.append(tail)
NSColor.white.setFill(); bubble.fill()
// Three typing dots.
NSColor(srgbRed: 0.07, green: 0.52, blue: 0.45, alpha: 1).setFill()
for (i, x) in [438.0, 512.0, 586.0].enumerated() {
    let d: CGFloat = 46
    NSBezierPath(ovalIn: CGRect(x: x - d / 2, y: 470 - d / 2 + (i == 1 ? 4 : 0), width: d, height: d)).fill()
}

// On-duty status light.
let dot = CGRect(x: 724, y: 724, width: 112, height: 112)
NSColor.white.setFill(); NSBezierPath(ovalIn: dot.insetBy(dx: -14, dy: -14)).fill()
NSColor(srgbRed: 1.0, green: 0.74, blue: 0.20, alpha: 1).setFill(); NSBezierPath(ovalIn: dot).fill()

NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: out)
