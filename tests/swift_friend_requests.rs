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
