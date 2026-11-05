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

func findTesseract() -> String? {
    var candidates: [String] = []
    if let explicit = environment("WECHAT_OCR_TESSERACT"), !explicit.isEmpty {
        candidates.append(explicit)
    }
    candidates.append(contentsOf: [
        "/opt/homebrew/bin/tesseract",
        "/usr/local/bin/tesseract",
    ])
    if let path = environment("PATH") {
        for dir in path.split(separator: ":") {
            candidates.append("\(dir)/tesseract")
        }
    }
    for candidate in candidates where FileManager.default.isExecutableFile(atPath: candidate) {
        return candidate
    }
    return nil
}

func runVision() -> [TextLine]? {
    let semaphore = DispatchSemaphore(value: 0)
    var output: [TextLine] = []
    var failure: Error?
    DispatchQueue.global(qos: .userInitiated).async {
        let request = VNRecognizeTextRequest()
        request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        let handler = VNImageRequestHandler(cgImage: image, options: [:])
        do {
            try handler.perform([request])
            for observation in request.results ?? [] {
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
            failure = error
        }
        semaphore.signal()
    }
    // Vision 精确模型在部分系统上会卡住，必须有超时，否则整个 MCP 调用会挂死。
    if semaphore.wait(timeout: .now() + .seconds(12)) == .timedOut {
        fputs("Vision 识别超时\n", stderr)
        return nil
    }
    if let failure {
        fputs("Vision 识别出错: \(failure)\n", stderr)
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

func containsCJK(_ text: String) -> Bool {
    text.unicodeScalars.contains { scalar in
        (0x4E00...0x9FFF).contains(scalar.value)
            || (0x3400...0x4DBF).contains(scalar.value)
            || (0xF900...0xFAFF).contains(scalar.value)
    }
}

func parseTesseractTSV(_ data: Data) -> [TextLine] {
    guard let raw = String(data: data, encoding: .utf8) else { return [] }
    struct Group {
        var left = Int.max
        var top = Int.max
        var right = Int.min
        var bottom = Int.min
        var words: [String] = []
    }
    var groups: [String: Group] = [:]
    for line in raw.split(separator: "\n", omittingEmptySubsequences: true) {
        let fields = line.split(separator: "\t", omittingEmptySubsequences: false)
        guard fields.count >= 12,
              Int(fields[0]) == 5,
              let left = Int(fields[6]),
              let top = Int(fields[7]),
              let width = Int(fields[8]),
              let height = Int(fields[9]) else { continue }
        let word = String(fields[11]).trimmingCharacters(in: .whitespaces)
        if word.isEmpty { continue }
        let key = "\(fields[2])/\(fields[3])/\(fields[4])"
        var group = groups[key] ?? Group()
        group.left = min(group.left, left)
        group.top = min(group.top, top)
        group.right = max(group.right, left + width)
        group.bottom = max(group.bottom, top + height)
        group.words.append(word)
        groups[key] = group
    }
    let imageWidth = Double(image.width)
    let imageHeight = Double(image.height)
    return groups.values.compactMap { group in
        var text = group.words.joined(separator: " ")
        if containsCJK(text) {
            text = text.replacingOccurrences(of: " ", with: "")
        }
        guard !text.isEmpty else { return nil }
        return TextLine(
            text: text,
            y: min(max(Double(group.top) / imageHeight, 0.0), 1.0),
            x: min(max(Double(group.left) / imageWidth, 0.0), 1.0)
        )
    }
}

func runTesseract() -> [TextLine]? {
    guard let executable = findTesseract() else {
        fputs("未找到 tesseract；macOS 26+ 可执行 brew install tesseract tesseract-lang\n", stderr)
        return nil
    }
    let tempURL = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("wechat_ocr_\(UUID().uuidString).png")
    guard writePNG(image, to: tempURL) else {
        fputs("无法写入 OCR 临时图片\n", stderr)
        return nil
    }
    defer { try? FileManager.default.removeItem(at: tempURL) }

    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    // 写文件而不是 stdout，避免大图 TSV 超过管道缓冲区导致 tesseract 阻塞。
    let outputBase = tempURL.deletingPathExtension()
    let tsvURL = URL(fileURLWithPath: outputBase.path + ".tsv")
    process.arguments = [tempURL.path, outputBase.path, "-l", "chi_sim+eng", "--psm", "6", "tsv"]
    let errorPipe = Pipe()
    process.standardError = errorPipe
    do {
        try process.run()
    } catch {
        fputs("启动 tesseract 失败: \(error)\n", stderr)
        return nil
    }
    let semaphore = DispatchSemaphore(value: 0)
    DispatchQueue.global(qos: .userInitiated).async {
        process.waitUntilExit()
        semaphore.signal()
    }
    if semaphore.wait(timeout: .now() + .seconds(20)) == .timedOut {
        process.terminate()
        fputs("tesseract 识别超时\n", stderr)
        return nil
    }
    guard process.terminationStatus == 0 else {
        let err = String(data: errorPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        fputs("tesseract 识别失败: \(err.prefix(300))\n", stderr)
        return nil
    }
    defer { try? FileManager.default.removeItem(at: tsvURL) }
    guard let data = try? Data(contentsOf: tsvURL) else {
        fputs("tesseract 未生成 TSV 输出\n", stderr)
        return nil
    }
    let lines = parseTesseractTSV(data)
    return lines.isEmpty ? nil : lines
}

do { try writeDebugEvidence() }
catch { fputs("\(error)\n", stderr); exit(1) }

/// Same switch file as the bot worker (~/.wechat-rpa/ocr.json or WECHAT_OCR_CONFIG):
/// missing or "vision" keeps the existing automatic choice; "rapidocr" tries RapidOCR first.
func switchFileEngine() -> String {
    let path = environment("WECHAT_OCR_CONFIG") ?? NSHomeDirectory() + "/.wechat-rpa/ocr.json"
    guard FileManager.default.fileExists(atPath: path) else { return "auto" }
    guard let data = FileManager.default.contents(atPath: path),
          let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let value = (object["engine"] as? String)?.lowercased(),
          ["vision", "rapidocr"].contains(value) else {
        fputs("invalid_ocr_engine: \(path)\n", stderr)
        exit(1)
    }
    return value == "rapidocr" ? "rapidocr" : "auto"
}

/// Runs native/rapidocr_server.py --once on the (cropped) image; nil with a reason on any failure.
func runRapidOCR() -> ([TextLine]?, String) {
    let python = environment("WECHAT_RAPIDOCR_PYTHON") ?? "/usr/bin/python3"
    let script = environment("WECHAT_RAPIDOCR_SCRIPT") ?? NSHomeDirectory() + "/.wechat-rpa/bin/rapidocr_server.py"
    guard FileManager.default.isExecutableFile(atPath: python), FileManager.default.fileExists(atPath: script) else {
        return (nil, "rapidocr_unavailable")
    }
    let folder = environment("WECHAT_OCR_TMP") ?? NSHomeDirectory() + "/.wechat-rpa/ocr-tmp"
    let path = folder + "/" + UUID().uuidString + ".png"
    defer { try? FileManager.default.removeItem(atPath: path) }
    do {
        try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    } catch { return (nil, "rapidocr_image_write_failed") }
    guard let destination = CGImageDestinationCreateWithURL(URL(fileURLWithPath: path) as CFURL, "public.png" as CFString, 1, nil) else {
        return (nil, "rapidocr_image_write_failed")
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { return (nil, "rapidocr_image_write_failed") }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: python)
    process.arguments = [script, "--once", path]
    let output = Pipe()
    process.standardOutput = output
    process.standardError = FileHandle.nullDevice
    var data = Data()
    let finished = DispatchSemaphore(value: 0)
    do { try process.run() } catch { return (nil, "rapidocr_unavailable") }
    DispatchQueue.global(qos: .userInitiated).async {
        data = output.fileHandleForReading.readDataToEndOfFile()
        finished.signal()
    }
    if finished.wait(timeout: .now() + .seconds(30)) == .timedOut {
        process.terminate()
        return (nil, "rapidocr_timeout")
    }
    process.waitUntilExit()
    let objects = String(decoding: data, as: UTF8.self).split(separator: "\n").compactMap {
        try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any]
    }
    if let error = objects.compactMap({ $0["error"] as? String }).first { return (nil, "rapidocr_error: " + error) }
    guard let lines = objects.compactMap({ $0["lines"] as? [[String: Any]] }).first else { return (nil, "rapidocr_bad_reply") }
    let width = Double(image.width), height = Double(image.height)
    // Same format as Vision/Tesseract here: normalised top-left corner of each line.
    let converted: [TextLine] = lines.compactMap { line in
        guard let text = line["text"] as? String, let box = line["box"] as? [[Double]], box.count == 4,
              box.allSatisfy({ $0.count == 2 }),
              !text.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        let left = box.map { $0[0] }.min() ?? 0, top = box.map { $0[1] }.min() ?? 0
        return TextLine(text: text, y: min(max(top / height, 0), 1), x: min(max(left / width, 0), 1))
    }
    return (converted, "")
}

var engine = (environment("WECHAT_OCR_ENGINE") ?? "").lowercased()
if engine.isEmpty { engine = switchFileEngine() }
guard ["auto", "vision", "tesseract", "rapidocr"].contains(engine) else {
    fputs("无效的 WECHAT_OCR_ENGINE\n", stderr)
    exit(1)
}
let majorVersion = ProcessInfo.processInfo.operatingSystemVersion.majorVersion
var recognized: [TextLine]?
if engine == "rapidocr" {
    let (lines, reason) = runRapidOCR()
    recognized = lines
    if lines == nil {
        fputs("ocr_fallback: \(reason)\n", stderr)
        engine = "auto"
    }
}
let tesseractFirst = engine == "tesseract" || (engine == "auto" && majorVersion >= 26)

if recognized != nil {
    // RapidOCR answered.
} else if tesseractFirst {
    recognized = runTesseract()
    if recognized?.isEmpty != false && engine != "tesseract" {
        if let fallback = runVision(), recognized == nil || !fallback.isEmpty {
            recognized = fallback
        }
    }
} else {
    recognized = runVision()
    if recognized?.isEmpty != false && engine != "vision" && findTesseract() != nil {
        if let fallback = runTesseract(), recognized == nil || !fallback.isEmpty {
            recognized = fallback
        }
    }
}

guard var lines = recognized else {
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
