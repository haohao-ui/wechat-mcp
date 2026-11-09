#[test]
fn ocr_rows_and_details() {
    let dir = std::env::temp_dir().join(format!("friend-ocr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.swift"),r#"
import Foundation
func l(_ t:String,_ x:Double,_ y:Double)->[String:Any] { ["text":t,"x":x,"y":y,"w":50.0,"h":12.0] }
let lines=[l("新的朋友",77,110),l("甲",133,160),l("等待验证",243,160),l("你好",133,184),l("乙",133,210),l("已添加",243,210),l("合作",133,234),l("公众号、服务号",77,260),l("越界",133,300),l("等待验证",243,300)]
let rows=parseFriendRequestRows(lines)
assert(rows.count == 2)
assert(rows[0]["name"] as? String == "甲")
assert(rows[0]["message"] as? String == "你好")
assert(rows[0]["status"] as? String == "等待验证")
let d=parseFriendRequestDetail([l("甲",430,100),l("你好",430,150),l("回复",430,200),l("来源通过搜索手机号添加",430,250),l("前往验证",559,328)])
assert(d["source"] as? String == "通过搜索手机号添加")
assert(d["message"] as? String == "你好")
assert(d["verify_button"] is [String:Double])
print("PASS: friend request OCR fixtures")
"#).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new("swiftc")
        .arg(root.join("native/bot/FriendRequests.swift"))
        .arg(dir.join("main.swift"))
        .arg("-o")
        .arg(dir.join("test"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new(dir.join("test"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

fn swift_fixture(name: &str, body: &str) {
    let dir = std::env::temp_dir().join(format!("friend-ocr-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        r#"import Foundation
func l(_ t:String,_ x:Double,_ y:Double)->[String:Any] {{ ["text":t,"x":x,"y":y,"w":50.0,"h":12.0] }}
{body}
"#
    );
    std::fs::write(dir.join("main.swift"), source).unwrap();
    let out = std::process::Command::new("swiftc")
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("native/bot/FriendRequests.swift"),
        )
        .arg(dir.join("main.swift"))
        .arg("-o")
        .arg(dir.join("test"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new(dir.join("test"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn status_ocr_confusion_still_pending() {
    swift_fixture(
        "status_confusion",
        r#"
let rows=parseFriendRequestRows([l("新的朋友",77,110),l("甲",133,160),l("簩待验证",243,160)])
assert(rows.count == 1)
assert(rows[0]["status"] as? String == "等待验证")
"#,
    );
}

#[test]
fn status_classification() {
    swift_fixture(
        "status",
        r#"
assert(friendRequestStatus("等待验证") == "等待验证")
assert(friendRequestStatus("已添加") == "已添加")
assert(friendRequestStatus("已过期") == "已过期")
assert(friendRequestStatus("验证") == nil)
assert(friendRequestStatus("其他文字") == nil)
assert(friendRequestStatus(" 等 待 验证\n") == "等待验证")
assert(friendRequestStatus("一二三四验证") == nil)
"#,
    );
}

#[test]
fn name_tolerates_one_char_ocr_difference() {
    swift_fixture(
        "names",
        r#"
assert(sameFriendName("红泥小火炉", "红泥小火妒"))
assert(!sameFriendName("甲", "乙"))
assert(sameFriendName("红 泥小火炉", "红泥小火炉"))
assert(sameFriendName("红泥小火炉", "红泥火炉"))
assert(sameFriendName("红泥火炉", "红泥小火炉"))
assert(!sameFriendName("红泥小火炉", "绿泥小火妒"))
assert(!sameFriendName("甲乙", "甲乙丙"))
"#,
    );
}

#[test]
fn ambiguous_similar_names_rejected() {
    swift_fixture(
        "ambiguous",
        r#"
let rows=parseFriendRequestRows([l("新的朋友",77,110),l("红泥小火妒",133,160),l("等待验证",243,160),l("红泥小火户",133,210),l("已添加",243,210)])
assert(uniqueFriendRequestRow(rows, "红泥小火炉") == nil)
assert(uniqueFriendRequestRow(Array(rows.prefix(1)), "红泥小火炉") != nil)
assert(uniqueFriendRequestRow(Array(rows.suffix(1)), "红泥小火炉") == nil)
"#,
    );
}
