import Foundation
import Vision
import ImageIO
import UniformTypeIdentifiers
import Darwin

guard CommandLine.arguments.count > 1 else {
    fputs("用法: wechat_ocr <图片路径> [--crop x y width height]（图片像素，左上角原点）\n", stderr)
    exit(1)
}

struct TextLine: Codable {
    let text: String
    let y: Double
    let x: Double
}

let imagePath = CommandLine.arguments[1]
let url = URL(fileURLWithPath: imagePath)

let image: CGImage
if CommandLine.arguments.count == 2 {
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let sourceImage = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
        fputs("无法读取图片: \(imagePath)\n", stderr)
        exit(1)
    }
    image = sourceImage
} else {
    guard CommandLine.arguments.count == 7, CommandLine.arguments[2] == "--crop",
          let x = Int(CommandLine.arguments[3]), let y = Int(CommandLine.arguments[4]),
          let width = Int(CommandLine.arguments[5]), let height = Int(CommandLine.arguments[6]),
          x >= 0, y >= 0, width > 0, height > 0,
          let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let sourceImage = CGImageSourceCreateImageAtIndex(source, 0, nil),
          x <= sourceImage.width, y <= sourceImage.height,
          width <= sourceImage.width - x, height <= sourceImage.height - y,
          let cropped = sourceImage.cropping(to: CGRect(x: x, y: y, width: width, height: height)) else {
        fputs("OCR 裁剪参数无效或无法读取图片（需使用图片实际像素坐标）\n", stderr)
        exit(1)
    }
    image = cropped
}

func environment(_ key: String) -> String? {
    ProcessInfo.processInfo.environment[key]
}

func runVision() -> [TextLine]? {
    let semaphore = DispatchSemaphore(value: 0)
    var output: [TextLine] = []
    var failureReason: String?
    DispatchQueue.global(qos: .userInitiated).async {
        let request = VNRecognizeTextRequest()
        request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        let handler = VNImageRequestHandler(cgImage: image, options: [:])
        do {
            try handler.perform([request])
            guard let results = request.results else {
                failureReason = "Vision 未返回识别结果 (results 为空)"
                semaphore.signal()
                return
            }
            for observation in results {
                guard let candidate = observation.topCandidates(1).first else { continue }
                let box = observation.boundingBox
                // Vision 的 Y 轴从底部向上；输出改为从上到下的归一化坐标。
                output.append(TextLine(
                    text: candidate.string,
                    y: 1.0 - box.origin.y - box.height,
                    x: box.origin.x
                ))
            }
        } catch {
            failureReason = "Vision 执行失败: \(error)"
        }
        semaphore.signal()
    }
    // Vision 精确模型在部分系统上会卡住，必须有超时，否则整个 MCP 调用会挂死。
    if semaphore.wait(timeout: .now() + .seconds(12)) == .timedOut {
        fputs("Vision 识别超时\n", stderr)
        return nil
    }
    if let failureReason = failureReason {
        fputs("Vision 识别出错: \(failureReason)\n", stderr)
        return nil
    }
    return output
}

func writePNG(_ image: CGImage, to url: URL) -> Bool {
    guard let destination = CGImageDestinationCreateWithURL(
        url as CFURL,
        UTType.png.identifier as CFString,
        1,
        nil
    ) else { return false }
    CGImageDestinationAddImage(destination, image, nil)
    return CGImageDestinationFinalize(destination)
}

// Explicitly enabled, private evidence makes coordinate/OCR failures inspectable.
func writeDebugEvidence() throws {
    let directory = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(".wechat-rpa/diagnostics", isDirectory: true)
    let enabled = directory.appendingPathComponent("capture.enabled")
    guard FileManager.default.fileExists(atPath: enabled.path) else { return }
    let attributes = try FileManager.default.attributesOfItem(atPath: directory.path)
    let flag = try FileManager.default.attributesOfItem(atPath: enabled.path)
    guard attributes[.type] as? FileAttributeType == .typeDirectory,
          (attributes[.ownerAccountID] as? NSNumber)?.uint32Value == getuid(),
          ((attributes[.posixPermissions] as? NSNumber)?.intValue ?? 0o777) & 0o077 == 0,
          flag[.type] as? FileAttributeType == .typeRegular,
          (flag[.ownerAccountID] as? NSNumber)?.uint32Value == getuid() else {
        throw NSError(domain: "OCR诊断目录权限不安全", code: 1)
    }
    let name = url.lastPathComponent
    let role = name.contains("title") ? "title" : (name.contains("search") ? "search" : (name.contains("list") || name.contains("session") ? "sessions" : "chat"))
    let output = directory.appendingPathComponent("\(role).png")
    guard writePNG(image, to: output) else { throw NSError(domain: "OCR诊断图片保存失败", code: 1) }
    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: output.path)
}

func validateOCREngine() {
    let envEngine = (environment("WECHAT_OCR_ENGINE") ?? "").lowercased()
    if !envEngine.isEmpty && envEngine != "vision" {
        fputs("无效的 WECHAT_OCR_ENGINE: \(envEngine)\n", stderr)
        exit(1)
    }
    let configPath = environment("WECHAT_OCR_CONFIG") ?? NSHomeDirectory() + "/.wechat-rpa/ocr.json"
    if FileManager.default.fileExists(atPath: configPath) {
        guard let data = FileManager.default.contents(atPath: configPath),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let value = (object["engine"] as? String)?.lowercased(),
              value == "vision" else {
            fputs("invalid_ocr_engine: \(configPath)\n", stderr)
            exit(1)
        }
    }
}

do { try writeDebugEvidence() }
catch { fputs("\(error)\n", stderr); exit(1) }

validateOCREngine()

guard var lines = runVision() else {
    fputs("OCR 引擎失败，无法取得识别结果\n", stderr)
    exit(1)
}

lines.sort { lhs, rhs in lhs.y == rhs.y ? lhs.x < rhs.x : lhs.y < rhs.y }

let encoder = JSONEncoder()
encoder.outputFormatting = .prettyPrinted
do {
    let data = try encoder.encode(lines)
    print(String(decoding: data, as: UTF8.self))
} catch {
    fputs("OCR 结果编码失败\n", stderr)
    exit(1)
}
