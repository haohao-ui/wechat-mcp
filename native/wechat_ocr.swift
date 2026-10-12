import Foundation
import Vision

guard CommandLine.arguments.count > 1 else {
    fputs("用法: wechat_ocr <图片路径>\n", stderr)
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
let handler = VNImageRequestHandler(url: url, options: [:])
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
