// Self-test driver for native/eyes/Anchors.swift.
import Foundation
import CoreGraphics
import ImageIO

func loadImage(_ path: String) -> CGImage? {
    let url = CFURLCreateWithFileSystemPath(nil, path as CFString, .cfurlposixPathStyle, false)
    guard let src = CGImageSourceCreateWithURL(url!, nil),
          let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
        return nil
    }
    return img
}

@main
struct AnchorsMain {
    static func main() {
        let code = run()
        if code != 0 {
            exit(code)
        }
    }

    static func run() -> Int32 {
        let args = CommandLine.arguments
        guard args.count >= 3 else {
            fputs("usage: anchors <png> <layout.json> [--shift dx,dy] [--blank <region>] [--patch x,y,w,h,r,g,b]\n", stderr)
            return 1
        }

        let pngPath = args[1]
        let layoutPath = args[2]

        guard let origImage = loadImage(pngPath) else {
            fputs("error: unable to load image from \(pngPath)\n", stderr)
            return 1
        }

        let layout: WechatLayout
        do {
            layout = try WechatLayout.load(path: layoutPath)
        } catch {
            fputs("error: unable to load layout: \(error)\n", stderr)
            return 1
        }

        let scale = origImage.width / Int(layout.window.width)
        let s = Double(scale)
        let w = origImage.width
        let h = origImage.height

        // Parse options
        var shiftDx: Double = 0
        var shiftDy: Double = 0
        var blankRegion: String? = nil
        var patchParams: (x: Double, y: Double, w: Double, h: Double, r: Double, g: Double, b: Double)? = nil

        var i = 3
        while i < args.count {
            let opt = args[i]
            if opt == "--shift" && i + 1 < args.count {
                let parts = args[i + 1].split(separator: ",").compactMap { Double($0) }
                if parts.count == 2 {
                    shiftDx = parts[0]
                    shiftDy = parts[1]
                }
                i += 2
            } else if opt == "--blank" && i + 1 < args.count {
                blankRegion = args[i + 1]
                i += 2
            } else if opt == "--patch" && i + 1 < args.count {
                let parts = args[i + 1].split(separator: ",").compactMap { Double($0) }
                if parts.count == 7 {
                    patchParams = (parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6])
                }
                i += 2
            } else {
                i += 1
            }
        }

        // Render modified image into a new bitmap context
        guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB),
              let ctx = CGContext(
                data: nil,
                width: w,
                height: h,
                bitsPerComponent: 8,
                bytesPerRow: w * 4,
                space: colorSpace,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue
              ) else {
            fputs("error: failed to create context\n", stderr)
            return 1
        }

        // Shift drawing if requested
        // Note: CoreGraphics coordinate system has origin at bottom-left when drawing.
        // In window coordinates, (0,0) is top-left, y goes down.
        // When translating window coordinates:
        // Window (x + dx, y + dy) means in CGContext (x + dx, y - dy).
        let drawRect = CGRect(x: shiftDx * s, y: -shiftDy * s, width: Double(w), height: Double(h))
        ctx.draw(origImage, in: drawRect)

        // Check if we need to blank a region
        if let regionName = blankRegion, let rect = layout.regions[regionName] {
            // Detect theme on original to determine background color
            let theme = detectTheme(origImage, scale: scale, layout: layout)
            let isLight = (theme == "wechat-4-light")
            let bg = isLight ? CGFloat(250.0 / 255.0) : CGFloat(30.0 / 255.0)
            ctx.setFillColor(red: bg, green: bg, blue: bg, alpha: 1.0)
            // Convert window rect (top-left origin) to CGContext rect (bottom-left origin)
            let cgY = Double(h) - (rect.y + rect.h) * s
            ctx.fill(CGRect(x: rect.x * s, y: cgY, width: rect.w * s, height: rect.h * s))
        }

        // Apply patch if requested
        if let p = patchParams {
            ctx.setFillColor(red: CGFloat(p.r / 255.0), green: CGFloat(p.g / 255.0), blue: CGFloat(p.b / 255.0), alpha: 1.0)
            let cgY = Double(h) - (p.y + p.h) * s
            ctx.fill(CGRect(x: p.x * s, y: cgY, width: p.w * s, height: p.h * s))
        }

        guard let finalImage = ctx.makeImage() else {
            fputs("error: failed to make image\n", stderr)
            return 1
        }

        let report = verifyAnchors(finalImage, scale: scale, layout: layout)
        do {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            let data = try encoder.encode(report)
            if let jsonStr = String(data: data, encoding: .utf8) {
                print(jsonStr)
                return 0
            }
        } catch {
            fputs("error encoding json: \(error)\n", stderr)
            return 1
        }

        return 0
    }
}
