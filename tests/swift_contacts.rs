use std::path::PathBuf;
use std::process::Command;

fn has_swiftc() -> bool {
    Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn build_contacts_test_bin(test_case: &str) -> Option<PathBuf> {
    if !has_swiftc() {
        if std::env::var("CI").is_ok() || std::env::var("WECHAT_STRICT_VISION").is_ok() {
            panic!("swiftc unavailable in strict mode");
        }
        eprintln!("skip: swiftc unavailable");
        return None;
    }

    let dir = std::env::temp_dir().join(format!("contacts-test-{}-{}", std::process::id(), test_case));
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.swift");

    let main_code = match test_case {
        "filter" => r#"
import Foundation
let spec = ContactsManagerFilterSpec(x: 40, tagY: 167, allY: 52)
// 全部：固定位置
guard let all = managerFilterPoint(kind: "all", y: nil, windowHeight: 560, spec: spec), all.x == 40, all.y == 52 else { print("FAIL: all"); exit(1) }
// 标签：必须由调用方给出识别到的 y，且在标签标题下方、窗口内
guard let tag = managerFilterPoint(kind: "tag", y: 208, windowHeight: 560, spec: spec), tag.x == 40, tag.y == 208 else { print("FAIL: tag 208"); exit(1) }
guard managerFilterPoint(kind: "tag", y: nil, windowHeight: 560, spec: spec) == nil else { print("FAIL: tag without y"); exit(1) }
guard managerFilterPoint(kind: "tag", y: 167, windowHeight: 560, spec: spec) == nil else { print("FAIL: tag header itself"); exit(1) }
guard managerFilterPoint(kind: "tag", y: 600, windowHeight: 560, spec: spec) == nil else { print("FAIL: tag outside window"); exit(1) }
guard managerFilterPoint(kind: "other", y: 208, windowHeight: 560, spec: spec) == nil else { print("FAIL: unknown kind"); exit(1) }
print("PASS: manager_filter_point_verified")
"#,
        "layout" => r#"
import Foundation

// 1. 正常解析带有 contacts 段的布局
let layoutPath = "native/layout/wechat-4.1.15.json"
guard let layout = try? WechatLayout.load(path: layoutPath) else {
    print("FAIL: failed to load layout from \(layoutPath)")
    exit(1)
}

do {
    let spec = try layout.contactsSpec()
    guard spec.navContacts.x == 30 && spec.navContacts.y == 169 else {
        print("FAIL: navContacts mismatch \(spec.navContacts)")
        exit(1)
    }
    guard spec.navChats.x == 30 && spec.navChats.y == 121 else {
        print("FAIL: navChats mismatch \(spec.navChats)")
        exit(1)
    }
    guard spec.list.x == 60 && spec.list.y == 40 && spec.list.w == 240 && spec.list.h == 600 else {
        print("FAIL: list mismatch \(spec.list)")
        exit(1)
    }
    guard spec.managerButton.x == 180 && spec.managerButton.y == 59 else {
        print("FAIL: managerButton mismatch \(spec.managerButton)")
        exit(1)
    }
    guard spec.profile.x == 420 && spec.profile.y == 50 && spec.profile.w == 400 && spec.profile.h == 390 else {
        print("FAIL: profile mismatch \(spec.profile)")
        exit(1)
    }
    guard spec.profileForbidden.x == 420 && spec.profileForbidden.y == 440 && spec.profileForbidden.w == 420 && spec.profileForbidden.h == 120 else {
        print("FAIL: profileForbidden mismatch \(spec.profileForbidden)")
        exit(1)
    }
    guard spec.rowHeight == 50 else {
        print("FAIL: rowHeight mismatch \(spec.rowHeight)")
        exit(1)
    }
    guard spec.manager.firstRowY == 96 && spec.manager.rowHeight == 48 && spec.manager.rows == 10 else {
        print("FAIL: manager mismatch \(spec.manager)")
        exit(1)
    }
    guard spec.manager.columns.nameX == 300 && spec.manager.columns.remarkX == 385 && spec.manager.columns.tagsX == 500 && spec.manager.columns.permissionX == 620 else {
        print("FAIL: manager columns mismatch \(spec.manager.columns)")
        exit(1)
    }
    guard spec.manager.filter.x == 40 && spec.manager.filter.tagY == 167 else {
        print("FAIL: manager filter mismatch \(spec.manager.filter)")
        exit(1)
    }
    print("PASS: layout_contacts_parsed")
} catch {
    print("FAIL: contactsSpec threw \(error)")
    exit(1)
}

// 2. 缺 contacts 段报 layout_missing_contacts
let rawData = try! Data(contentsOf: URL(fileURLWithPath: layoutPath))
var dict = try! JSONSerialization.jsonObject(with: rawData) as! [String: Any]
dict.removeValue(forKey: "contacts")
let strippedData = try! JSONSerialization.data(withJSONObject: dict)
let strippedFile = FileManager.default.temporaryDirectory.appendingPathComponent("stripped-\(ProcessInfo.processInfo.processIdentifier).json")
try! strippedData.write(to: strippedFile)

guard let strippedLayout = try? WechatLayout.load(path: strippedFile.path) else {
    print("FAIL: stripped layout failed to load")
    exit(1)
}
do {
    _ = try strippedLayout.contactsSpec()
    print("FAIL: expected missing contacts error but succeeded")
    exit(1)
} catch let err as LayoutError {
    if err.description == "layout_missing_contacts" {
        print("PASS: layout_missing_contacts")
    } else {
        print("FAIL: unexpected error \(err.description)")
        exit(1)
    }
} catch {
    print("FAIL: unexpected non-LayoutError \(error)")
    exit(1)
}
"#,
        "forbidden" => r#"
import Foundation

let layoutPath = "native/layout/wechat-4.1.15.json"
guard let layout = try? WechatLayout.load(path: layoutPath) else {
    print("FAIL: failed to load layout")
    exit(1)
}

// contactsClickAllowed(point:(480,496), layout) == false
let forbidden1 = contactsClickAllowed(point: (480.0, 496.0), layout: layout)
guard !forbidden1 else {
    print("FAIL: (480, 496) should be forbidden")
    exit(1)
}

// (180,230) == true
let allowed1 = contactsClickAllowed(point: (180.0, 230.0), layout: layout)
guard allowed1 else {
    print("FAIL: (180, 230) should be allowed")
    exit(1)
}

// list 区外 == false
let outside1 = contactsClickAllowed(point: (10.0, 10.0), layout: layout)
guard !outside1 else {
    print("FAIL: (10, 10) should be outside list")
    exit(1)
}
let outside2 = contactsClickAllowed(point: (350.0, 200.0), layout: layout)
guard !outside2 else {
    print("FAIL: (350, 200) should be outside list")
    exit(1)
}

// 资料卡禁点区过滤：y >= 440 的行必须被剔除
let mockLines: [[String: Any]] = [
    ["text": "微信号: wxid_123", "x": 430.0, "y": 60.0, "w": 200.0, "h": 20.0],
    ["text": "地区: 中国", "x": 430.0, "y": 100.0, "w": 150.0, "h": 20.0],
    ["text": "发消息", "x": 480.0, "y": 480.0, "w": 100.0, "h": 40.0],
    ["text": "音视频通话", "x": 600.0, "y": 480.0, "w": 100.0, "h": 40.0]
]
let spec = try! layout.contactsSpec()
let filtered = filterProfileForbiddenLines(lines: mockLines, forbiddenRect: spec.profileForbidden)
guard filtered.count == 2 else {
    print("FAIL: expected 2 lines after filtering forbidden region, got \(filtered.count)")
    exit(1)
}
guard filtered.allSatisfy({ ($0["text"] as? String) != "发消息" && ($0["text"] as? String) != "音视频通话" }) else {
    print("FAIL: forbidden buttons still present in lines: \(filtered)")
    exit(1)
}

print("PASS: forbidden_click_guard_verified")
"#,
        "pick" => r#"
import Foundation

// 主窗口 880x640，管理窗口 720x560 标题"通讯录管理"
let mainWin: [String: Any] = [
    "window_id": UInt32(100),
    "title": "微信",
    "width": 880.0,
    "height": 640.0,
    "x": 100.0,
    "y": 100.0
]
let managerWin: [String: Any] = [
    "window_id": UInt32(101),
    "title": "通讯录管理",
    "width": 720.0,
    "height": 560.0,
    "x": 200.0,
    "y": 150.0
]

// 选中管理窗口
let picked = pickManagerWindow([mainWin, managerWin])
guard let pickedWin = picked else {
    print("FAIL: pickManagerWindow returned nil when manager present")
    exit(1)
}
guard pickedWin["window_id"] as? UInt32 == 101 || (pickedWin["title"] as? String) == "通讯录管理" else {
    print("FAIL: pickManagerWindow selected wrong window: \(pickedWin)")
    exit(1)
}

// 只有主窗口时返回 nil
let pickedNone = pickManagerWindow([mainWin])
guard pickedNone == nil else {
    print("FAIL: pickManagerWindow should return nil for only main window")
    exit(1)
}

print("PASS: manager_window_pick_verified")
"#,
        _ => panic!("unknown test case"),
    };

    std::fs::write(&main, main_code).unwrap();
    let bin = dir.join(format!("contacts_test_{}", test_case));

    let mut cmd = Command::new("swiftc");
    cmd.arg("native/common/Geometry.swift")
        .arg("native/common/WechatLayout.swift")
        .arg("native/bot/Contacts.swift")
        .arg(&main)
        .arg("-o")
        .arg(&bin);

    let out = cmd.output().expect("failed to execute swiftc");
    if !out.status.success() {
        eprintln!(
            "swiftc compilation failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(bin)
}

#[test]
fn layout_contacts_section_parses() {
    let Some(bin) = build_contacts_test_bin("layout") else {
        panic!("build contacts test failed");
    };
    let out = Command::new(&bin).output().expect("failed to run test bin");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "run failed: {}", stdout);
    assert!(stdout.contains("PASS: layout_contacts_parsed"), "stdout: {stdout}");
    assert!(stdout.contains("PASS: layout_missing_contacts"), "stdout: {stdout}");
}

#[test]
fn forbidden_click_guard_rejects_profile_buttons() {
    let Some(bin) = build_contacts_test_bin("forbidden") else {
        panic!("build contacts test failed");
    };
    let out = Command::new(&bin).output().expect("failed to run test bin");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "run failed: {}", stdout);
    assert!(stdout.contains("PASS: forbidden_click_guard_verified"), "stdout: {stdout}");
}

#[test]
fn manager_window_pick_prefers_titled_manager() {
    let Some(bin) = build_contacts_test_bin("pick") else {
        panic!("build contacts test failed");
    };
    let out = Command::new(&bin).output().expect("failed to run test bin");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "run failed: {}", stdout);
    assert!(stdout.contains("PASS: manager_window_pick_verified"), "stdout: {stdout}");
}

/// 标签筛选的点击位置不能靠"标题 y + 序号 × 行高"推算（实测标签间距与表格行高不同，序号 0 会点到"标签"标题本身）。
#[test]
fn manager_filter_point_requires_observed_tag_y() {
    let Some(bin) = build_contacts_test_bin("filter") else {
        panic!("build contacts test failed");
    };
    let out = Command::new(&bin).output().expect("failed to run test bin");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "run failed: {}", stdout);
    assert!(stdout.contains("PASS: manager_filter_point_verified"), "stdout: {stdout}");
}
