import Foundation
import ScreenCaptureKit
import CoreMedia
import CoreImage
import CryptoKit

final class WindowCapture: NSObject, SCStreamOutput, SCStreamDelegate {
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "wechat.capture", qos: .userInitiated)
    private let context = CIContext(options: [.cacheIntermediates: false])
    private var stream: SCStream?
    private var selected: RuntimeWindow?
    private var latest: CGImage?
    private var latestAt: UInt64 = 0
    private var lastFrameUptime: TimeInterval = 0
    private var failure: String?
    private var epoch = UUID().uuidString
    private var sequence: UInt64 = 0
    private var activeStreamIdentity: ObjectIdentifier?

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        lock.lock(); defer { lock.unlock() }
        guard activeStreamIdentity == ObjectIdentifier(stream) else { return }
        failure = "capture_stream_stopped"
        latest = nil
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, buffer.isValid,
              let attachments = CMSampleBufferGetSampleAttachmentsArray(buffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let attachment = attachments.first,
              let raw = attachment[.status] as? Int,
              let status = SCFrameStatus(rawValue: raw) else { return }
        lock.lock(); defer { lock.unlock() }
        guard activeStreamIdentity == ObjectIdentifier(stream) else { return }
        lastFrameUptime = ProcessInfo.processInfo.systemUptime
        if status == .idle { return }
        guard status == .complete else { latest = nil; return }
        guard let pixels = CMSampleBufferGetImageBuffer(buffer),
              let image = context.createCGImage(CIImage(cvPixelBuffer: pixels), from: CGRect(x: 0, y: 0, width: CVPixelBufferGetWidth(pixels), height: CVPixelBufferGetHeight(pixels))) else {
            failure = "capture_frame_invalid"; latest = nil; return
        }
        latest = image
        latestAt = UInt64(Date().timeIntervalSince1970 * 1000)
        sequence += 1
    }

    private func frame() -> (CGImage?, UInt64, UInt64, String?, TimeInterval) {
        lock.lock(); defer { lock.unlock() }
        return (latest, sequence, latestAt, failure, lastFrameUptime)
    }

    func image(window: RuntimeWindow, refresh: Bool) async throws -> (CGImage, String, UInt64, UInt64) {
        guard #available(macOS 14.0, *) else { throw RuntimeFailure("native_stream_requires_macos14") }
        guard CGPreflightScreenCaptureAccess() else { throw RuntimeFailure("screen_recording_permission_missing") }
        let existing = frame()
        if selected != window || refresh || stream == nil || ProcessInfo.processInfo.systemUptime - existing.4 > 2 {
            let identityChanged = selected != window
            try await stop()
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            guard let target = content.windows.first(where: { $0.windowID == window.window_id && $0.owningApplication?.processID == window.pid }) else {
                throw RuntimeFailure("capture_window_changed")
            }
            let config = SCStreamConfiguration()
            let scale = content.displays.max(by: {
                $0.frame.intersection(target.frame).area < $1.frame.intersection(target.frame).area
            }).map { Double($0.width) / $0.frame.width } ?? 1
            let boundedScale = min(max(scale, 1), 4096 / max(window.width, window.height))
            config.width = max(1, Int(window.width * boundedScale))
            config.height = max(1, Int(window.height * boundedScale))
            config.pixelFormat = kCVPixelFormatType_32BGRA
            config.minimumFrameInterval = CMTime(value: 1, timescale: 5)
            config.queueDepth = 3
            config.showsCursor = false
            config.ignoreShadowsSingleWindow = true
            config.capturesAudio = false
            let next = SCStream(filter: SCContentFilter(desktopIndependentWindow: target), configuration: config, delegate: self)
            try next.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
            lock.withLock {
                latest = nil; failure = nil
                lastFrameUptime = 0
                if identityChanged { sequence = 0; epoch = UUID().uuidString }
                activeStreamIdentity = ObjectIdentifier(next)
            }
            stream = next
            selected = window
            try await next.startCapture()
        }
        for _ in 0..<100 {
            let current = frame()
            if let failure = current.3 { throw RuntimeFailure(failure) }
            if let image = current.0 { return (image, epoch, current.1, current.2) }
            try await Task.sleep(nanoseconds: 30_000_000)
        }
        throw RuntimeFailure("capture_frame_timeout")
    }

    func stop() async throws {
        lock.withLock { activeStreamIdentity = nil; latest = nil }
        if let active = stream { try await active.stopCapture() }
        stream = nil
        selected = nil
        lock.withLock { latest = nil; failure = nil }
    }
}

private extension CGRect {
    var area: CGFloat { isNull ? 0 : width * height }
}

func imageDigest(_ image: CGImage) throws -> String {
    guard let data = image.dataProvider?.data else { throw RuntimeFailure("capture_pixels_missing") }
    var hash = SHA256()
    hash.update(data: Data("\(image.width):\(image.height):\(image.bytesPerRow):\(image.bitsPerPixel):".utf8))
    hash.update(data: data as Data)
    return hash.finalize().map { String(format: "%02x", $0) }.joined()
}
