import Foundation

/// Continuity identity of a file card: filename and size token without whitespace, so OCR spacing
/// ("2024最新" vs "2024 最新") or a status change in place never makes the same card look new.
func fileCardIdentity(filename: String, size: String) -> String {
    let squeeze: (String) -> String = { $0.components(separatedBy: .whitespacesAndNewlines).joined() }
    return "file:" + squeeze(filename) + ":" + squeeze(size).uppercased()
}
