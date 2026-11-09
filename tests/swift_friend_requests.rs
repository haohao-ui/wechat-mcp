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
let rows=parseFriendRequestRows([l("新的朋友",77,110),l("甲",133,160),l("甲待验证",243,160)])
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

#[test]
fn closed_vocabulary_consensus() {
    swift_fixture(
        "consensus",
        r#"
let original=Array("等待验证")
for index in original.indices {
 var changed=original; changed[index]=Character(UnicodeScalar(0x7530+index)!)
 let read=String(changed)
 assert(friendWordConsensus(read,read,.status).word == "等待验证")
}
assert(friendWordConsensus("已添甲","已添甲",.status).word == nil)
assert(friendWordConsensus("取甲","取甲",.button).word == nil)
assert(friendWordConsensus("等待验证","已添加",.status).word == nil)
assert(friendWordConsensus("通过搜索甲号添加","通过搜索甲号添加",.source).word == nil)
assert(friendWordConsensus("通过甲甲手机号添加","通过甲甲手机号添加",.source).word == nil)
for index in [0,1,8] {
 var changed=Array("通过搜索手机号添加"); changed[index]=Character(UnicodeScalar(0x7530+index)!)
 assert(friendWordConsensus(String(changed),String(changed),.source).word == "通过搜索手机号添加")
}
"#,
    );
}

#[test]
fn unknown_ui_fields_are_redacted_and_not_actionable() {
    swift_fixture(
        "unknown_fields",
        r#"
var status=l("private unreadable text",243,160)
status["fr_checked"]=true;status["fr_kind"]="status";status["fr_scores"]="best=0.500,second=0.400"
let rows=parseFriendRequestRows([l("新的朋友",77,110),l("甲",133,160),status],requireConsensus:true)
assert(rows.count==1 && rows[0]["status"] as? String == "unknown")
assert(rows[0]["error"] as? String == "unknown_status best=0.500,second=0.400")
var source=l("来源private unreadable text",430,250)
source["fr_checked"]=true;source["fr_kind"]="source";source["fr_scores"]="best=0.500,second=0.400"
let detail=parseFriendRequestDetail([l("甲",430,100),source,l("前往验证",559,328)])
assert(detail["source"] as? String == "")
assert(detail["error"] as? String == "unknown_source best=0.500,second=0.400")
var button=l("private button text",559,328)
button["fr_checked"]=true;button["fr_kind"]="button";button["fr_scores"]="best=0.500,second=0.400"
let blocked=parseFriendRequestDetail([l("来源通过搜索手机号添加",430,250),button])
assert(blocked["verify_button"] is NSNull)
assert(blocked["error"] as? String == "unknown_button best=0.500,second=0.400")
"#,
    );
}

/// Layout observed on 2026-11-09 (360x640 「通过朋友验证」 window), with the OCR variance seen there.
const OBSERVED_DIALOG: &str = r#"
func dialog(_ extra:[[String:Any]]=[],tag:String="搜奈或创建标签.",toggle:String="不让他（她）看",drop:String="")->[[String:Any]] {
 ([l("通过朋友验证",137,17),l("备注",34,66),l("甲乙丙丁戊",35,103),l("标签",35,152),l(tag,32,190),l("朋友权限",36,248),
  l("聊天、朋友圈、微信运动等",57,286),l("仅聊天",60,331),l("朋友圈和状态",35,378),l(toggle,34,416),l("不看他（她）",32,460),
  l("确定",233,593),l("取消",97,594)]+extra).filter{($0["text"] as? String) != drop}
}
"#;

#[test]
fn verification_dialog_observed_layout_is_accepted() {
    swift_fixture(
        "dialog_ok",
        &format!(
            r#"{OBSERVED_DIALOG}
let ok=classifyFriendDialog(dialog())
assert(ok["error"] == nil,"\(ok)")
assert((ok["cancel"] as? [String:Any])?["x"] as? Double == 97)
assert((ok["confirm"] as? [String:Any])?["x"] as? Double == 233)
// Half-width parentheses are the same text after compatibility normalization.
assert(classifyFriendDialog(dialog(toggle:"不让他(她)看"))["error"] == nil)
// Field contents are exempt by position, whatever OCR reads there.
assert(classifyFriendDialog(dialog(tag:"任意内容"))["error"] == nil)
"#
        ),
    );
}

#[test]
fn verification_dialog_unexpected_content_needs_review() {
    swift_fixture(
        "dialog_unknown",
        &format!(
            r#"{OBSERVED_DIALOG}
// An unobserved line outside the input fields blocks any click.
let extra=classifyFriendDialog(dialog([l("请输入验证码",40,520)]))
assert((extra["error"] as? String)?.hasPrefix("unknown_dialog") == true && extra["confirm"] == nil)
// A misread label is unknown, never corrected.
assert(classifyFriendDialog(dialog(toggle:"不甲他乙她丙看"))["error"] != nil)
assert(classifyFriendDialog(dialog(drop:"通过朋友验证"))["error"] != nil)
assert(classifyFriendDialog(dialog(drop:"朋友权限"))["error"] != nil)
assert(classifyFriendDialog(dialog(drop:"取消"))["error"] != nil)
assert(classifyFriendDialog(dialog([l("确定",233,560)]))["error"] != nil)
"#
        ),
    );
}
