import Foundation
import Vision

final class TextRecognizer {
    private var cache: [String: (String, [RuntimeText])] = [:]

    func recognize(_ image: CGImage, region: String, digest: String) throws -> ([RuntimeText], Bool) {
        if let (previous, lines) = cache[region], previous == digest { return (lines, true) }
        let request = VNRecognizeTextRequest()
        request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
        let lines = (request.results ?? []).compactMap { item -> RuntimeText? in
            guard let candidate = item.topCandidates(1).first,
                  !candidate.string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            let box = item.boundingBox
            return RuntimeText(text: candidate.string,
                bounds: RegionRect(x: max(0, box.minX), y: max(0, 1 - box.maxY),
                                   width: min(box.width, 1 - max(0, box.minX)), height: min(box.height, box.maxY)),
                confidence: Double(candidate.confidence))
        }.sorted { $0.bounds.y == $1.bounds.y ? $0.bounds.x < $1.bounds.x : $0.bounds.y < $1.bounds.y }
        cache[region] = (digest, lines)
        return (lines, false)
    }
}
