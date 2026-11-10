//! 附件保存时访问受保护文件夹（桌面/文稿/下载）可能因系统权限弹窗在内核里无限阻塞，
//! 并一直占着桌面锁。bounded() 必须在时限内返回明确错误，而不是跟着阻塞。
use std::path::PathBuf;
use std::process::Command;

fn has_swiftc() -> bool {
    Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn build_selftest() -> Option<PathBuf> {
    if !has_swiftc() {
        if std::env::var("CI").is_ok() || std::env::var("WECHAT_STRICT_VISION").is_ok() {
            panic!("swiftc unavailable in strict mode");
        }
        eprintln!("skip: swiftc unavailable");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("bounded-selftest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.swift");
    std::fs::write(
        &main,
        r#"
import Foundation
// 1. 正常完成：返回值原样带回
let v = try! bounded(seconds: 2, label: "Fast") { 42 }
print("fast=\(v)")
// 2. 操作抛错：错误原样带回
struct E: Error {}
do { _ = try bounded(seconds: 2, label: "Throws") { () throws -> Int in throw E() }; print("throws=none") }
catch is E { print("throws=E") } catch { print("throws=other:\(error)") }
// 3. 阻塞超过时限：按时返回带文件夹名的错误
let began = Date()
do { _ = try bounded(seconds: 0.5, label: "Desktop") { () -> Int in Thread.sleep(forTimeInterval: 30); return 1 }; print("slow=none") }
catch { print("slow=\(error) after=\(Int(Date().timeIntervalSince(began) * 1000))") }
"#,
    )
    .unwrap();
    let bin = dir.join("bounded_selftest");
    let out = Command::new("swiftc")
        .arg("native/bot/Bounded.swift")
        .arg(&main)
        .arg("-o")
        .arg(&bin)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "swiftc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(bin)
}

#[test]
fn bounded_returns_value_propagates_error_and_times_out_with_folder_label() {
    let Some(bin) = build_selftest() else { return };
    let out = Command::new(&bin).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("fast=42"), "{text}");
    assert!(text.contains("throws=E"), "{text}");
    let slow = text
        .lines()
        .find(|l| l.starts_with("slow="))
        .unwrap_or_default();
    assert!(slow.contains("folder_access_blocked:Desktop"), "{slow}");
    let ms: u64 = slow
        .rsplit("after=")
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        (400..3000).contains(&ms),
        "超时应在约 0.5 秒返回，实际 {ms}ms"
    );
}

fn save_attachment_source() -> String {
    let src = std::fs::read_to_string("native/bot/Files.swift").unwrap();
    let start = src.find("func saveAttachment(").expect("saveAttachment");
    let rest = &src[start..];
    let end = rest.find("\nfunc ").unwrap_or(rest.len());
    rest[..end].to_string()
}

/// 点任何界面元素之前先在时限内探测候选文件夹：权限弹窗未处理时直接报错，不去点微信。
#[test]
fn save_attachment_preflights_folders_before_any_click() {
    let body = save_attachment_source();
    let preflight = body
        .find("preflightFolders(")
        .expect("saveAttachment 必须调用 preflightFolders");
    // The right-click lives in the shared chooseContextMenu; its call is the first click here.
    let first_click = body
        .find("chooseContextMenu(")
        .expect("saveAttachment 通过 chooseContextMenu 右键");
    assert!(preflight < first_click, "文件夹探测必须在右键点击之前");
}

/// 轮询下载结果时，对候选文件夹里文件的访问都必须有时限。
#[test]
fn save_attachment_polling_never_touches_files_without_time_limit() {
    let body = save_attachment_source();
    let loop_start = body.find("while Date()<deadline").expect("polling loop");
    let polling = &body[loop_start..];
    for raw in [
        "FileManager.default.fileExists(atPath:found.path)",
        "FileManager.default.attributesOfItem(atPath:found.path)",
        "FileManager.default.moveItem(at:found",
    ] {
        for (i, _) in polling.match_indices(raw) {
            let line_start = polling[..i].rfind('\n').map(|n| n + 1).unwrap_or(0);
            let line = &polling[line_start
                ..polling[i..]
                    .find('\n')
                    .map(|n| i + n)
                    .unwrap_or(polling.len())];
            // 同一行带 bounded(，或位于一个以 bounded(...) { 开头、尚未闭合的块里
            let before = &polling[..line_start];
            let in_block = before.rfind("bounded(").map_or(false, |b| {
                let opened = &polling[b..line_start];
                opened.matches('{').count() > opened.matches('}').count()
            });
            assert!(
                line.contains("bounded(") || in_block,
                "未加时限的文件访问: {line}"
            );
        }
    }
}
