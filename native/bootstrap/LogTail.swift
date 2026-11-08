// LogTail.swift: Efficient incremental log reader tailing file ends.
import Foundation

public final class LogTail {
    public let url: URL
    public let maxLines: Int
    private var lastOffset: UInt64 = 0
    private var pendingRemainder: String = ""

    public init(url: URL, maxLines: Int = 500) {
        self.url = url
        self.maxLines = maxLines
    }

    /// 读取自上次 poll 以来新增的日志行；文件截断或轮换后自动从头读取；单次返回最多保留最近 maxLines 行。
    public func poll() -> [String] {
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: url.path),
              let currentSize = attrs[.size] as? UInt64 else {
            // 文件不存在或不可读
            lastOffset = 0
            pendingRemainder = ""
            return []
        }

        // 文件被截断或轮转
        if currentSize < lastOffset {
            lastOffset = 0
            pendingRemainder = ""
        }

        guard currentSize > lastOffset else {
            return []
        }

        guard let handle = try? FileHandle(forReadingFrom: url) else {
            return []
        }
        defer {
            try? handle.close()
        }

        do {
            try handle.seek(toOffset: lastOffset)
            let data = handle.readDataToEndOfFile()
            lastOffset = currentSize

            guard !data.isEmpty else { return [] }
            guard let chunk = String(data: data, encoding: .utf8) ?? String(data: data, encoding: .ascii) else {
                return []
            }

            let fullChunk = pendingRemainder + chunk
            var rawLines = fullChunk.components(separatedBy: "\n")
            if fullChunk.hasSuffix("\n") {
                _ = rawLines.popLast()
                pendingRemainder = ""
            } else {
                pendingRemainder = rawLines.popLast() ?? ""
            }

            // 清理末尾可能存在的 \r
            let cleanedLines = rawLines.map { line -> String in
                if line.hasSuffix("\r") {
                    return String(line.dropLast())
                }
                return line
            }

            if cleanedLines.count > maxLines {
                return Array(cleanedLines.suffix(maxLines))
            }
            return cleanedLines
        } catch {
            return []
        }
    }
}
