import Foundation

@main
struct WechatRuntime {
    static func main() async {
        let capture = WindowCapture()
        let recognizer = TextRecognizer()
        do {
            let layout = try WorkspaceLayout(path: ProcessInfo.processInfo.environment["WECHAT_MCP_DESKTOP_PROFILE"])
            if CommandLine.arguments.dropFirst() == ["--self-test"] {
                let rect = RegionRect(x: 0.25, y: 0.125, width: 0.5, height: 0.5)
                let crop = try pixelCrop(rect, width: 2000, height: 1000)
                guard crop.minX == 500, crop.minY == 125, crop.width == 1000, crop.height == 500 else {
                    throw RuntimeFailure("pixel_transform_self_test_failed")
                }
                do {
                    try RegionRect(x: 0.8, y: 0, width: 0.5, height: 1).validate()
                    throw RuntimeFailure("invalid_region_self_test_failed")
                } catch let error as RuntimeFailure where error.description == "invalid_region" {}
                try writeResponse(id: 0, result: ["self_test": "passed", "desktop_accessed": false])
                return
            }
            while let line = await Task.detached(operation: { readLine() }).value {
                var id: UInt64 = 0
                do {
                    guard let data = line.data(using: .utf8), data.count <= runtimeFrameLimit,
                          let request = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                          let requestID = request["id"] as? UInt64 else { throw RuntimeFailure("invalid_request") }
                    id = requestID
                    guard request["protocol_version"] as? Int == runtimeProtocolVersion,
                          let operation = request["operation"] as? String,
                          let params = request["params"] as? [String: Any] else { throw RuntimeFailure("protocol_mismatch") }
                    switch operation {
                    case "diagnostics":
                        try writeResponse(id: id, result: runtimeDiagnostics())
                    case "snapshot":
                        guard let name = params["region"] as? String,
                              let refresh = params["refresh"] as? Bool else { throw RuntimeFailure("invalid_snapshot_params") }
                        let window = try currentWechatWindow()
                        let region = try layout.region(name, window: window)
                        let (image, epoch, sequence, capturedAt) = try await capture.image(window: window, refresh: refresh)
                        let crop = try pixelCrop(region, width: image.width, height: image.height)
                        guard let cropped = image.cropping(to: crop) else { throw RuntimeFailure("capture_crop_failed") }
                        let digest = try imageDigest(cropped)
                        let (lines, cached) = try recognizer.recognize(cropped, region: name, digest: digest)
                        var titleLines: [RuntimeText] = []
                        if name == "chat" {
                            let titleRegion = try layout.region("title", window: window)
                            let titleCrop = try pixelCrop(titleRegion, width: image.width, height: image.height)
                            guard let titleImage = image.cropping(to: titleCrop) else { throw RuntimeFailure("title_crop_failed") }
                            titleLines = try recognizer.recognize(titleImage, region: "title", digest: imageDigest(titleImage)).0
                        }
                        guard try currentWechatWindow() == window else { throw RuntimeFailure("window_changed_during_observation") }
                        // Return the actual pixel-rounded ROI, so click mapping uses the same crop.
                        let actual = RegionRect(x: crop.minX / Double(image.width), y: crop.minY / Double(image.height),
                                                width: crop.width / Double(image.width), height: crop.height / Double(image.height))
                        let snapshot = RuntimeSnapshot(epoch: epoch, sequence: sequence, window: window,
                                                       region: actual, digest: digest, captured_at_ms: capturedAt,
                                                       ocr_cached: cached, lines: lines, title_lines: titleLines)
                        try writeResponse(id: id, result: jsonObject(snapshot))
                    default:
                        throw RuntimeFailure("unsupported_operation")
                    }
                } catch {
                    let message = (error as? RuntimeFailure)?.description ?? "native_operation_failed"
                    try writeResponse(id: id, error: message)
                }
            }
            try await capture.stop()
        } catch {
            fputs("wechat_runtime: initialization_or_transport_failed\n", stderr)
            exit(1)
        }
    }
}
