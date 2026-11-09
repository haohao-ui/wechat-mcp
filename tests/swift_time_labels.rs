//! Time separators recorded on chat bubbles (native/wechat_bubbles.swift `attachTimeLabels`).

#[test]
fn time_separators_attach_to_the_bubble_below() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("native/wechat_bubbles.swift")).unwrap();
    let function = source
        .split("/// Time separators")
        .nth(1)
        .unwrap()
        .split("/// End time separators")
        .next()
        .unwrap();
    let dir = std::env::temp_dir().join(format!("time-labels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Layout from the 2026-11-09 dedicated-Mac capture: 580pt chat pane, separators centered.
    let body = format!(
        r##"import Foundation
struct Text {{ let body: String; let x: Double; let y: Double }}
/// Time separators{function}
func b(_ y:Int)->[String:Any] {{ ["text":"开放接口验收测试","x":60,"y":y,"width":130,"height":36] }}
var bubbles=[b(41),b(139),b(237),b(335)]
let texts=[Text(body:"14:40",x:287,y:63-60),Text(body:"开放接口验收测试",x:137,y:51),
 Text(body:"19:42",x:286,y:163-40),Text(body:"21:30",x:286,y:221),Text(body:"昨天 21:41",x:290,y:320),
 Text(body:"12:00",x:80,y:300)]
attachTimeLabels(&bubbles,texts:texts,width:580)
assert(bubbles[0]["time_label"] as? String == "14:40")
assert(bubbles[1]["time_label"] as? String == "19:42")
assert(bubbles[2]["time_label"] as? String == "21:30")
// Prefixed separators are kept verbatim; an off-center time (inside a message) is ignored.
assert(bubbles[3]["time_label"] as? String == "昨天 21:41")
var none=[b(41)]
attachTimeLabels(&none,texts:[Text(body:"21:3O",x:287,y:20)],width:580)
assert(none[0]["time_label"] == nil)
print("PASS")
"##
    );
    std::fs::write(dir.join("main.swift"), body).unwrap();
    let out = std::process::Command::new("swiftc")
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
