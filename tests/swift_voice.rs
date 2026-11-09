//! Voice bubbles (native/wechat_bubbles.swift `markVoiceBubbles`), geometry and OCR readings
//! observed on the dedicated Mac on 2026-11-09.

fn run(name: &str, body: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("native/wechat_bubbles.swift")).unwrap();
    let function = source
        .split("/// Voice messages")
        .nth(1)
        .unwrap()
        .split("/// End voice messages")
        .next()
        .unwrap();
    let dir = std::env::temp_dir().join(format!("voice-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = format!(
        r##"import Foundation
struct Text {{ let body: String; let x: Double; let y: Double }}
/// Voice messages{function}
func bubble(_ text:String,_ x:Int,_ y:Int,_ w:Int,_ h:Int=37,dir:String="inbound_visual")->[String:Any] {{
 ["text":text,"x":x,"y":y,"width":w,"height":h,"direction":dir,"kind":"text","digest":"d\(y)"]
}}
// Icon ink only in the left 30pt of a bubble that starts at x=50 (group) or x=55.
let icon: (Int,Int)->Bool = {{ x,_ in (60...78).contains(x) }}
{body}
print("PASS")
"##
    );
    std::fs::write(dir.join("main.swift"), program).unwrap();
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

#[test]
fn observed_voice_readings_are_voice_with_duration() {
    run(
        "readings",
        r#"
// Every OCR reading recorded for real voice bubbles today, with the measured bubble width.
for (read,w,secs) in [("\")3\"",98,3),("•) 6\"",125,6),("•)) 5\"",116,5),(") 5\"",116,5),("合\"",125,6),("•））ぶ\"",116,5),("•)) 2\"",89,2)] {
 var b=[bubble(read,50,253,w)]
 markVoiceBubbles(&b,texts:[Text(body:read,x:90,y:271)],ink:icon)
 assert(b[0]["kind"] as? String == "voice","\(read)")
 assert(b[0]["voice_seconds"] as? Int == secs,"\(read)")
 assert(b[0]["media_id"] as? String == "d253")
}
// A long voice whose width no longer follows the formula still qualifies on a clean reading.
var long=[bubble("•)) 59\"",50,253,300)]
markVoiceBubbles(&long,texts:[Text(body:"•)) 59\"",x:90,y:271)],ink:icon)
assert(long[0]["kind"] as? String == "voice" && long[0]["voice_seconds"] as? Int == 59)
"#,
    );
}

#[test]
fn transcript_bubble_below_is_folded_into_the_voice() {
    run(
        "transcript",
        r#"
// Group capture 23:29: voice @253+37, transcript @294+37 (x 55), then the bot's reply on the right.
var b=[bubble(") 5\"",50,253,116),bubble("你好你好你好你好你好你好你好。",55,294,234),bubble("回复",400,347,200,77,dir:"outbound_visual")]
let texts=[Text(body:") 5\"",x:90,y:271),Text(body:"你好你好你好你好你好你好你好。",x:170,y:312)]
markVoiceBubbles(&b,texts:texts,ink:icon)
assert(b.count==2)
assert(b[0]["kind"] as? String == "voice")
assert(b[0]["transcript"] as? String == "你好你好你好你好你好你好你好。")
assert(b[0]["digest"] as? String == "d253", "voice pixels keep continuity")
// A normal text a message-gap below (>14pt) is not a transcript.
var far=[bubble(") 5\"",50,253,116),bubble("在吗",55,320,60)]
markVoiceBubbles(&far,texts:[Text(body:") 5\"",x:90,y:271)],ink:icon)
assert(far.count==2 && far[0]["transcript"] == nil)
"#,
    );
}

#[test]
fn ordinary_messages_are_not_voice() {
    run(
        "not_voice",
        r#"
for (read,w) in [("在吗",60),("5\"",98),("ok\"",50),("今天发货吗\"",120),("123",98),("好的哈\"",300)] {
 var b=[bubble(read,50,253,w)]
 markVoiceBubbles(&b,texts:[Text(body:read,x:90,y:271)],ink:read=="5\"" ? {_,_ in false} : icon)
 assert(b[0]["kind"] as? String == "text","\(read)")
}
// Outbound and multi-line bubbles never qualify.
var o=[bubble(") 5\"",400,253,116,dir:"outbound_visual"),bubble(") 5\"",50,300,116,77)]
markVoiceBubbles(&o,texts:[Text(body:") 5\"",x:450,y:271),Text(body:") 5\"",x:90,y:330)],ink:icon)
assert(o.allSatisfy { $0["kind"] as? String == "text" })
"#,
    );
}
