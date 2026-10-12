import Foundation
import Vision
import ImageIO

guard CommandLine.arguments.count > 1 else {
    fputs("用法: wechat_ocr <图片路径> [--crop x y width height]（图片像素，左上角原点）\n", stderr)
    exit(1)
}

let imagePath = CommandLine.arguments[1]
let url = URL(fileURLWithPath: imagePath)

struct TextLine: Codable {
    let text: String
    let y: Double
    let x: Double
}

var lines: [TextLine] = []
let handler: VNImageRequestHandler
if CommandLine.arguments.count == 2 {
    handler = VNImageRequestHandler(url: url, options: [:])
} else {
    guard CommandLine.arguments.count == 7, CommandLine.arguments[2] == "--crop",
          let x = Int(CommandLine.arguments[3]), let y = Int(CommandLine.arguments[4]),
          let width = Int(CommandLine.arguments[5]), let height = Int(CommandLine.arguments[6]),
          x >= 0, y >= 0, width > 0, height > 0,
          let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
          x <= image.width, y <= image.height,
          width <= image.width - x, height <= image.height - y,
          let cropped = image.cropping(to: CGRect(x: x, y: y, width: width, height: height)) else {
        fputs("OCR 裁剪参数无效或无法读取图片（需使用图片实际像素坐标）\n", stderr)
        exit(1)
    }
    // OCR 坐标相对于裁剪后的图片，与旧的区域截图行为一致。
    handler = VNImageRequestHandler(cgImage: cropped, options: [:])
}
let request = VNRecognizeTextRequest { (req, error) in
    guard let observations = req.results as? [VNRecognizedTextObservation] else { return }
    for obs in observations {
        guard let candidate = obs.topCandidates(1).first else { continue }
        let box = obs.boundingBox
        // Vision 坐标系 Y 轴从底部向上，转换为顶部向下 (0.0 表示最顶端，1.0 表示最底端)
        let topY = 1.0 - box.origin.y - box.height
        lines.append(TextLine(text: candidate.string, y: topY, x: box.origin.x))
    }
}

request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true

do {
    try handler.perform([request])
} catch {
    fputs("Vision 识别出错: \(error)\n", stderr)
    exit(1)
}

// 按纵向从上到下排序
lines.sort { $0.y < $1.y }

let encoder = JSONEncoder()
encoder.outputFormatting = .prettyPrinted
if let data = try? encoder.encode(lines), let json = String(data: data, encoding: .utf8) {
    print(json)
} else {
    print("[]")
}
