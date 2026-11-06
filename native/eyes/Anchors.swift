// Anchors.swift
// Screen anchor verification and recalibration measurements.
import Foundation
import CoreGraphics

public struct AnchorReport: Codable, Equatable {
    public let passed: Bool
    public let failures: [String]
    public let theme: String?
    public let measured: [String: Double]

    public init(passed: Bool, failures: [String], theme: String?, measured: [String: Double]) {
        self.passed = passed
        self.failures = failures
        self.theme = theme
        self.measured = measured
    }
}

/// Helper to extract RGBA pixel buffer from a CGImage.
private final class PixelBuffer {
    let width: Int
    let height: Int
    var data: [UInt8]

    init?(image: CGImage) {
        self.width = image.width
        self.height = image.height
        self.data = [UInt8](repeating: 0, count: width * height * 4)
        let rendered = data.withUnsafeMutableBytes { buf -> Bool in
            guard let ctx = CGContext(
                data: buf.baseAddress,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue
            ) else { return false }
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard rendered else { return nil }
    }

    @inline(__always)
    func rgb(x: Int, y: Int) -> (r: Int, g: Int, b: Int) {
        let idx = (y * width + x) * 4
        return (Int(data[idx]), Int(data[idx + 1]), Int(data[idx + 2]))
    }

    @inline(__always)
    func gray(x: Int, y: Int) -> Double {
        let idx = (y * width + x) * 4
        return (Double(data[idx]) * 299.0 + Double(data[idx + 1]) * 587.0 + Double(data[idx + 2]) * 114.0) / 1000.0
    }
}

/// Detects theme by sampling background pixels in chat_content.
public func detectTheme(_ image: CGImage, scale: Int, layout: WechatLayout) -> String? {
    guard let pixels = PixelBuffer(image: image) else { return nil }
    let chatRect = layout.regions["chat_content"] ?? layout.regions["chat"]
    guard let region = chatRect else { return nil }

    let pxStart = max(0, Int(round(region.x * Double(scale))))
    let pxEnd = min(pixels.width, Int(round((region.x + region.w) * Double(scale))))
    let pyStart = max(0, Int(round(region.y * Double(scale))))
    let pyEnd = min(pixels.height, Int(round((region.y + region.h) * Double(scale))))

    var samples: [(r: Int, g: Int, b: Int)] = []
    let step = max(1, 101 / scale)
    var count = 0
    for y in stride(from: pyStart, to: pyEnd, by: step) {
        for x in stride(from: pxStart, to: pxEnd, by: step) {
            samples.append(pixels.rgb(x: x, y: y))
            count += 1
            if count >= 1000 { break }
        }
        if count >= 1000 { break }
    }
    guard !samples.isEmpty else { return nil }

    let half = samples.count / 2
    for (name, theme) in layout.themes {
        if let bgMax = theme.backgroundMax {
            let matches = samples.filter { $0.r < bgMax && $0.g < bgMax && $0.b < bgMax }.count
            if matches > half { return name }
        }
        if let bgMin = theme.backgroundMin {
            let matches = samples.filter { $0.r >= bgMin && $0.g >= bgMin && $0.b >= bgMin }.count
            if matches > half { return name }
        }
    }
    return nil
}

/// Locates a single horizontal line spanning >= 70% length of the given rect with stable contrast to both sides.
/// Returns the window point y-coordinate, or nil if not found.
public func locateHorizontalLine(_ image: CGImage, scale: Int, in rect: WindowRect) -> Double? {
    guard let pixels = PixelBuffer(image: image) else { return nil }
    let pxStart = max(0, Int(round(rect.x * Double(scale))))
    let pxEnd = min(pixels.width, Int(round((rect.x + rect.w) * Double(scale))))
    let pyStart = max(0, Int(round(rect.y * Double(scale))))
    let pyEnd = min(pixels.height, Int(round((rect.y + rect.h) * Double(scale))))
    let pw = pxEnd - pxStart
    guard pw > 0 && pyEnd > pyStart else { return nil }

    let step = scale
    var bestY: Double? = nil
    var bestScore: Double = 0

    for py in pyStart..<pyEnd {
        if py - step < 0 || py + step >= pixels.height { continue }

        var validCount = 0
        var totalResponse: Double = 0

        for px in pxStart..<pxEnd {
            let c = pixels.gray(x: px, y: py)
            let top = pixels.gray(x: px, y: py - step)
            let bot = pixels.gray(x: px, y: py + step)

            let ridge = abs(2.0 * c - top - bot)
            let edge = abs(top - bot)
            let resp = max(ridge, edge)

            if resp >= 4.0 {
                validCount += 1
                totalResponse += resp
            }
        }

        let ratio = Double(validCount) / Double(pw)
        if ratio >= 0.70 {
            let avgResp = totalResponse / Double(pw)
            let score = ratio * 100.0 + avgResp
            if score > bestScore {
                bestScore = score
                bestY = Double(py) / Double(scale)
            }
        }
    }

    return bestY
}

/// Locates a single vertical line spanning >= 70% length of the given rect with stable contrast to both sides.
/// Returns the window point x-coordinate, or nil if not found.
public func locateVerticalLine(_ image: CGImage, scale: Int, in rect: WindowRect, minRatio: Double = 0.70) -> Double? {
    guard let pixels = PixelBuffer(image: image) else { return nil }
    let pxStart = max(0, Int(round(rect.x * Double(scale))))
    let pxEnd = min(pixels.width, Int(round((rect.x + rect.w) * Double(scale))))
    let pyStart = max(0, Int(round(rect.y * Double(scale))))
    let pyEnd = min(pixels.height, Int(round((rect.y + rect.h) * Double(scale))))
    let ph = pyEnd - pyStart
    guard ph > 0 && pxEnd > pxStart else { return nil }

    let step = scale
    var bestX: Double? = nil
    var bestScore: Double = 0

    for px in pxStart..<pxEnd {
        if px - step < 0 || px + step >= pixels.width { continue }

        var validCount = 0
        var totalResponse: Double = 0

        for py in pyStart..<pyEnd {
            let c = pixels.gray(x: px, y: py)
            let prev = pixels.gray(x: px - step, y: py)
            let next = pixels.gray(x: px + step, y: py)

            // Step gradient at boundary px vs px-step
            let grad = abs(c - prev)
            // Ridge line response
            let ridge = abs(2.0 * c - prev - next)
            let resp = max(grad, ridge)

            if resp >= 4.0 {
                validCount += 1
                totalResponse += resp
            }
        }

        let ratio = Double(validCount) / Double(ph)
        if ratio >= minRatio {
            let avgResp = totalResponse / Double(ph)
            let score = ratio * 100.0 + avgResp
            if score > bestScore {
                bestScore = score
                bestX = Double(px) / Double(scale)
            }
        }
    }

    return bestX
}

/// Checks whether text is present in the region by thresholding non-background pixels.
public func textPresent(_ image: CGImage, scale: Int, in rect: WindowRect) -> Bool {
    guard let pixels = PixelBuffer(image: image) else { return false }
    let pxStart = max(0, Int(round(rect.x * Double(scale))))
    let pxEnd = min(pixels.width, Int(round((rect.x + rect.w) * Double(scale))))
    let pyStart = max(0, Int(round(rect.y * Double(scale))))
    let pyEnd = min(pixels.height, Int(round((rect.y + rect.h) * Double(scale))))
    let total = (pxEnd - pxStart) * (pyEnd - pyStart)
    guard total > 0 else { return false }

    // Count color frequency in 16-level quantized space to find dominant background
    var hist: [Int: Int] = [:]
    for py in pyStart..<pyEnd {
        for px in pxStart..<pxEnd {
            let c = pixels.rgb(x: px, y: py)
            let key = ((c.r >> 3) << 10) | ((c.g >> 3) << 5) | (c.b >> 3)
            hist[key, default: 0] += 1
        }
    }
    guard let dominant = hist.max(by: { $0.value < $1.value }) else { return false }
    let bgR = ((dominant.key >> 10) & 0x1F) << 3
    let bgG = ((dominant.key >> 5) & 0x1F) << 3
    let bgB = (dominant.key & 0x1F) << 3

    var fgCount = 0
    for py in pyStart..<pyEnd {
        for px in pxStart..<pxEnd {
            let c = pixels.rgb(x: px, y: py)
            let dist = abs(c.r - bgR) + abs(c.g - bgG) + abs(c.b - bgB)
            if dist >= 20 {
                fgCount += 1
            }
        }
    }

    let fgRatio = Double(fgCount) / Double(total)
    // Non-background pixel threshold: >= 0.5% indicates text present
    return fgRatio >= 0.005
}

/// Verifies layout anchors against the normalized window frame.
public func verifyAnchors(_ image: CGImage, scale: Int, layout: WechatLayout, tolerance: Double = 3) -> AnchorReport {
    var failures: [String] = []
    var measured: [String: Double] = [:]

    // 1. Detect theme
    let detectedTheme = detectTheme(image, scale: scale, layout: layout)
    if detectedTheme == nil {
        failures.append("theme")
    }

    // 2. Check each anchor
    for anchor in layout.anchors {
        guard let region = layout.regions[anchor.region] else {
            failures.append(anchor.name)
            continue
        }

        switch anchor.kind {
        case "horizontal_line":
            let expectedY: Double
            if anchor.edge == "top" {
                expectedY = region.y
            } else {
                expectedY = region.y + region.h
            }
            // Band of +/- 12 points around the expected edge
            let band = WindowRect(x: region.x, y: expectedY - 12, w: region.w, h: 24)
            if let lineY = locateHorizontalLine(image, scale: scale, in: band) {
                measured[anchor.name] = lineY
                if abs(lineY - expectedY) > tolerance {
                    failures.append(anchor.name)
                }
            } else {
                failures.append(anchor.name)
            }

        case "vertical_line":
            let expectedX: Double
            if anchor.edge == "left" {
                expectedX = region.x
            } else {
                expectedX = region.x + region.w
            }
            // Band of +/- 12 points around the expected edge
            let band = WindowRect(x: expectedX - 12, y: region.y, w: 24, h: region.h)
            if let lineX = locateVerticalLine(image, scale: scale, in: band) {
                measured[anchor.name] = lineX
                if abs(lineX - expectedX) > tolerance {
                    failures.append(anchor.name)
                }
            } else {
                failures.append(anchor.name)
            }

        case "text_present":
            if !textPresent(image, scale: scale, in: region) {
                failures.append(anchor.name)
            }

        default:
            failures.append(anchor.name)
        }
    }

    let passed = failures.isEmpty
    return AnchorReport(passed: passed, failures: failures, theme: detectedTheme, measured: measured)
}

/// Where each line anchor really is, searched widely (re-measuring after a WeChat update, when the map
/// may be far off): horizontal edges from 30% into the region down to 40 points above the window
/// bottom (top edges: window top to 70% into the region); vertical edges from 30% to 180% of the
/// region width (left edges: mirrored). Text anchors have no position and are left out.
public func measureAnchors(_ image: CGImage, scale: Int, layout: WechatLayout) -> [String: Double] {
    let windowW = layout.window.width, windowH = layout.window.height
    var measured: [String: Double] = [:]
    for anchor in layout.anchors {
        guard let region = layout.regions[anchor.region] else { continue }
        switch anchor.kind {
        case "horizontal_line":
            let top = anchor.edge == "top" ? 0 : region.y + region.h * 0.3
            let bottom = anchor.edge == "top" ? region.y + region.h * 0.7 : windowH - 40
            guard bottom > top else { continue }
            if let y = firstFullHorizontalLine(image, scale: scale, in: WindowRect(x: region.x, y: top, w: region.w, h: bottom - top)) {
                measured[anchor.name] = y
            }
        case "vertical_line":
            let left = anchor.edge == "left" ? max(0, region.x - region.w * 0.8) : region.x + region.w * 0.3
            let right = anchor.edge == "left" ? region.x + region.w * 0.7 : min(windowW - 40, region.x + region.w * 1.8)
            guard right > left else { continue }
            if let x = locateVerticalLine(image, scale: scale, in: WindowRect(x: left, y: region.y, w: right - left, h: region.h), minRatio: fullLineRatio) {
                measured[anchor.name] = x
            }
        default:
            continue
        }
    }
    return measured
}

/// A layout boundary spans the whole region (fixtures: 0.96-0.97 of the chat width); bubble, image and
/// toolbar edges stay below 0.8, except lines inside the input box, which lie below the divider.
let fullLineRatio = 0.9

/// The topmost row in `rect` whose step or ridge response covers `fullLineRatio` of the width.
func firstFullHorizontalLine(_ image: CGImage, scale: Int, in rect: WindowRect) -> Double? {
    guard let pixels = PixelBuffer(image: image) else { return nil }
    let pxStart = max(0, Int(round(rect.x * Double(scale))))
    let pxEnd = min(pixels.width, Int(round((rect.x + rect.w) * Double(scale))))
    let pyStart = max(scale, Int(round(rect.y * Double(scale))))
    let pyEnd = min(pixels.height - scale, Int(round((rect.y + rect.h) * Double(scale))))
    let width = pxEnd - pxStart
    guard width > 0 && pyEnd > pyStart else { return nil }
    var runStart: Int? = nil, runEnd: Int? = nil
    for py in pyStart..<pyEnd {
        var covered = 0
        for px in pxStart..<pxEnd {
            let c = pixels.gray(x: px, y: py)
            let top = pixels.gray(x: px, y: py - scale)
            let bottom = pixels.gray(x: px, y: py + scale)
            if max(abs(2.0 * c - top - bottom), abs(top - bottom)) >= 4.0 { covered += 1 }
        }
        if Double(covered) / Double(width) >= fullLineRatio {
            if runStart == nil { runStart = py }
            runEnd = py
        } else if let start = runStart, let end = runEnd {
            // The line and its two edges form a short run of rows (fixtures: 495-498); use its middle.
            return (Double(start + end) / 2 / Double(scale)).rounded(.down)
        }
    }
    if let start = runStart, let end = runEnd { return (Double(start + end) / 2 / Double(scale)).rounded(.down) }
    return nil
}
