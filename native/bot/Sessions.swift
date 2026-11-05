// Read-only session-list probe: lets the bot switch chats only when an allowlisted row changes.
import AppKit
import ApplicationServices
import Foundation
import CoreGraphics
import CryptoKit

/// Trims whitespace and zero-width characters only; same rule as Python `clean_title`.
func cleanTitle(_ raw: String) -> String {
    let invisible: Set<Unicode.Scalar> = Set((0x200B...0x200F).compactMap(Unicode.Scalar.init) + [Unicode.Scalar(0x2060)!, Unicode.Scalar(0xFEFF)!])
    let kept = String(String.UnicodeScalarView(raw.unicodeScalars.filter { !invisible.contains($0) }))
    return kept.trimmingCharacters(in:.whitespacesAndNewlines)
}

/// Set per worker request: the open conversation is an allowlisted group.
var groupTitles = false

/// A group title reads "示例群 (3)"; the member count is not part of the name. Private titles are exact.
func titleMatches(_ raw: String, _ accepted: Set<String>) -> Bool {
    let title = cleanTitle(raw)
    if accepted.contains(title) { return true }
    guard groupTitles, let range = title.range(of:#"\s*[(（]\d+[)）]$"#, options:.regularExpression) else { return false }
    return accepted.contains(String(title[..<range.lowerBound]))
}

func isBadgeRed(_ r: Int, _ g: Int, _ b: Int) -> Bool { r >= 200 && g <= 90 && b <= 90 }

/// OCR punctuation flips with the row's selection highlight (〕/]/】, ，/,, …/.); compare letters and digits only.
func stablePreview(_ text: String) -> String {
    String(text.unicodeScalars.filter { CharacterSet.letters.contains($0) || CharacterSet.decimalDigits.contains($0) }
        .map(Character.init))
}

struct SessionRow { var name: String; var y: Double; var time = ""; var preview: [String] = [] }

/// Groups OCR lines into rows: a left-aligned line starts a row; nearby lines are its time/preview.
func groupSessionRows(_ texts: [Text], width: Double) -> [SessionRow] {
    var rows: [SessionRow] = []
    for text in texts.sorted(by:{ $0.y < $1.y }) {
        let body = text.body.trimmingCharacters(in:.whitespaces)
        if body.isEmpty { continue }
        if let index = rows.indices.last, text.y - rows[index].y < 34 {
            if text.x > width*0.75 && abs(text.y - rows[index].y) < 10 && rows[index].time.isEmpty {
                rows[index].time = body
            } else {
                rows[index].preview.append(body)
            }
        } else if text.x <= width*0.75 {
            rows.append(SessionRow(name:body, y:text.y))
        }
    }
    return rows
}

final class SessionProbe {
    var cachedHash = ""
    var cachedRows: [SessionRow] = []

    func probe(_ full: CGImage, names: [String], debug: Bool = false) throws -> [String:Any] {
        let layout = try currentLayout()
        let sessionListRect = try layoutRegion(layout, "session_list")
        guard full.width == Int(layout.window.width), full.height == Int(layout.window.height), let list = full.cropping(to:sessionListRect) else {
            throw Failure.invalid("unsupported_image_geometry")
        }
        let width = list.width, height = list.height
        let pixels = try canonicalBytes(list)
        let hash = SHA256.hash(data:pixels).map { String(format:"%02x",$0) }.joined()
        let reused = hash == cachedHash
        if !reused {
            cachedRows = groupSessionRows(try recognize(list), width:Double(width))
            cachedHash = hash
        }
        func redPixels(top: Int, bottom: Int) -> Int {
            var count = 0
            // Avatar column only: the unread badge sits on the avatar's top-right corner.
            for y in max(0,top)..<min(height,bottom) {
                for x in 0..<min(width,80) {
                    let i = (y*width+x)*4
                    if isBadgeRed(Int(pixels[i]),Int(pixels[i+1]),Int(pixels[i+2])) { count += 1 }
                }
            }
            return count
        }
        let wanted = Set(names.map(cleanTitle))
        var found: [[String:Any]] = []
        for row in cachedRows where wanted.contains(cleanTitle(row.name)) {
            let summary = ([row.time]+row.preview).joined(separator:"\n")
            let digest = SHA256.hash(data:Data(stablePreview(summary).utf8)).map { String(format:"%02x",$0) }.joined()
            // preview_key (letters/digits of the preview only) lets the bot tell a one-letter OCR misread
            // from a new message; the time is returned apart and compared exactly.
            var item: [String:Any] = ["name":row.name, "y":Int(row.y), "preview_digest":digest,
                          "preview_key":stablePreview(row.preview.joined(separator:"\n")), "preview_time":row.time,
                          "red_pixels":redPixels(top:Int(row.y)-30, bottom:Int(row.y)+30)]
            if debug { item["debug_summary"] = summary }
            found.append(item)
        }
        // Only allowlisted rows leave the worker; other contacts' names/previews stay local.
        return ["profile":"wechat-4", "rows":found, "row_count":cachedRows.count,
                "list_digest":hash, "ocr_cached":reused]
    }
}

func titleNames(_ full: CGImage) throws -> [String] {
    let layout = try currentLayout()
    let titleRect = try layoutRegion(layout, "title")
    guard full.width == Int(layout.window.width), full.height == Int(layout.window.height),
          let title = full.cropping(to:titleRect) else {
        throw Failure.invalid("unsupported_image_geometry")
    }
    return try recognize(title).filter { $0.x >= 50 && $0.x < 220 }.map(\.body)
}

/// Opens an allowlisted chat by clicking its session-list row, then verifies the exact title.
/// Any doubt throws, and the caller falls back to the slower MCP navigation.
func openSessionRow(_ targets: [String], probe: SessionProbe, capture: () throws -> ([String:Any], CGImage)) throws -> [String:Any] {
    let accepted = Set(targets.map(cleanTitle))
    func isTarget(_ titles: [String]) -> Bool { titles.count == 1 && titleMatches(titles[0], accepted) }
    let began = ProcessInfo.processInfo.systemUptime
    guard AXIsProcessTrusted() else { throw Failure.invalid("post_event_not_allowed") }
    let (window, image) = try capture()
    if isTarget(try titleNames(image)) { return ["opened":true, "clicked":false, "seconds":0.0] }
    _ = try probe.probe(image, names:targets)
    let matches = probe.cachedRows.filter { accepted.contains(cleanTitle($0.name)) }
    guard matches.count == 1, let row = matches.first else { throw Failure.invalid("session_row_not_unique_or_missing") }
    let layout = try currentLayout()
    let sessionListRect = try layoutRegion(layout, "session_list")
    let x = window["x"] as! Double, y = window["y"] as! Double
    let local = CGPoint(x:sessionListRect.midX, y:sessionListRect.minY + row.y + layout.sessionRow.nameOffset)
    // The press lands up to 12pt from this point (effective region of the 40pt row target): keep it in the list.
    guard sessionListRect.insetBy(dx:0, dy:12).contains(local) else { throw Failure.invalid("session_row_out_of_bounds") }
    let point = CGPoint(x:x + local.x, y:y + local.y)
    // The topmost window under the point must be WeChat's: never click into another app.
    let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
    let hit = rows.first { entry in
        guard entry[kCGWindowLayer as String] as? Int == 0,
              let bounds = entry[kCGWindowBounds as String] as? [String:Any],
              let rect = CGRect(dictionaryRepresentation:bounds as CFDictionary) else { return false }
        return rect.contains(point)
    }
    guard let hit = hit, hit[kCGWindowNumber as String] as? UInt32 == window["window_id"] as? UInt32 else {
        throw Failure.invalid("wechat_row_not_topmost")
    }
    if let app = NSRunningApplication(processIdentifier:(hit[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value ?? 0),
       !app.isActive {
        app.activate(options:[])
        Thread.sleep(forTimeInterval:0.15)
    }
    // The row's name line: list width, 40pt around the point (effective region ±12pt, never the next row).
    let list = sessionListRect.offsetBy(dx:x, dy:y)
    try act { try botActions.click(CGRect(x:x+sessionListRect.minX, y:point.y-20, width:sessionListRect.width, height:40), within:list) }
    let deadline = began + 2.5
    while ProcessInfo.processInfo.systemUptime < deadline {
        Thread.sleep(forTimeInterval:0.15)
        let (_, after) = try capture()
        if isTarget(try titleNames(after)) {
            return ["opened":true, "clicked":true, "seconds":ProcessInfo.processInfo.systemUptime - began]
        }
    }
    throw Failure.invalid("title_not_verified_after_click")
}
