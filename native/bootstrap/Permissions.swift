// Permissions.swift: Permission detection, request, and System Settings navigation.
import Foundation
import AppKit
import ApplicationServices
import CoreGraphics

public enum PermissionKind: String, CaseIterable {
    case screen
    case accessibility
    case desktop
    case documents
    case downloads
}

public func permissionStatus(_ k: PermissionKind) -> Bool? {
    switch k {
    case .screen:
        if #available(macOS 10.15, *) {
            return CGPreflightScreenCaptureAccess()
        }
        return true
    case .accessibility:
        return AXIsProcessTrusted()
    case .desktop:
        let home = FileManager.default.homeDirectoryForCurrentUser
        let url = home.appendingPathComponent("Desktop")
        return checkFolderAccess(url: url)
    case .documents:
        let home = FileManager.default.homeDirectoryForCurrentUser
        let url = home.appendingPathComponent("Documents")
        return checkFolderAccess(url: url)
    case .downloads:
        let home = FileManager.default.homeDirectoryForCurrentUser
        let url = home.appendingPathComponent("Downloads")
        return checkFolderAccess(url: url)
    }
}

private func checkFolderAccess(url: URL) -> Bool? {
    var isDir: ObjCBool = false
    guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue else {
        return nil
    }
    // 尝试列出目录内容
    do {
        _ = try FileManager.default.contentsOfDirectory(atPath: url.path)
        return true
    } catch {
        return false
    }
}

public func requestPermission(_ k: PermissionKind) {
    switch k {
    case .screen:
        if #available(macOS 10.15, *) {
            _ = CGRequestScreenCaptureAccess()
        }
    case .accessibility:
        let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        _ = AXIsProcessTrustedWithOptions(options)
    case .desktop, .documents, .downloads:
        // 尝试触发 TCC 权限弹框
        let home = FileManager.default.homeDirectoryForCurrentUser
        let folderName = (k == .desktop) ? "Desktop" : (k == .documents ? "Documents" : "Downloads")
        let url = home.appendingPathComponent(folderName)
        _ = try? FileManager.default.contentsOfDirectory(atPath: url.path)
        // 若依然未授权，引导打开系统设置
        if permissionStatus(k) != true {
            openSystemSettings(for: k)
        }
    }
}

public func openSystemSettings(for k: PermissionKind) {
    let urlString: String
    switch k {
    case .screen:
        urlString = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
    case .accessibility:
        urlString = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
    case .desktop, .documents, .downloads:
        urlString = "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders"
    }

    if let url = URL(string: urlString) {
        NSWorkspace.shared.open(url)
    }
}
