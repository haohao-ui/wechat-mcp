import Foundation
import CoreGraphics

final class TestSink: EventSink {
    var position = CGPoint(x:20,y:20)
    var events: [[String:Any]] = []
    var cancelled = false
    var cancelOnDrag = false
    var failKeyDown: CGKeyCode?
    func cursor() -> CGPoint { position }
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton) throws {
        events.append(["type":type.rawValue,"x":Double(point.x),"y":Double(point.y)])
        position = point
        if cancelOnDrag && type == .leftMouseDragged { cancelled = true }
    }
    func post(_ type: CGEventType, at point: CGPoint, button: CGMouseButton, clickCount: Int) throws {
        try post(type,at:point,button:button)
        events[events.count-1]["count"] = clickCount
    }
    func scroll(lines: Int32, at point: CGPoint) throws {}
    func key(_ code: CGKeyCode, down: Bool, command: Bool) throws {
        try key(code,down:down,modifiers:command ? .maskCommand : [])
    }
    func key(_ code: CGKeyCode, down: Bool, modifiers: CGEventFlags) throws {
        events.append(["key":Int(code),"down":down,"flags":modifiers.rawValue])
        if down && code == failKeyDown { throw ActionError.eventFailed }
    }
    func sleep(_ seconds: Double) {}
}

let screen = CGRect(x:0,y:0,width:1000,height:800)
var results: [String:Bool] = [:]
let doubleSink = TestSink()
try Actions(sink:doubleSink,seed:10,screens:[screen]).doubleClick(CGRect(x:200,y:120,width:80,height:40),interval:0.08)
let presses = doubleSink.events.filter { $0["count"] != nil }
results["native_double_click"] = presses.compactMap { $0["count"] as? Int } == [1,1,2,2]
    && Set(presses.compactMap { $0["x"] as? Double }).count == 1
results["human_trajectory_preserved"] = doubleSink.events.filter { $0["type"] as? UInt32 == CGEventType.mouseMoved.rawValue }.count > 10

let dragSink = TestSink()
try Actions(sink:dragSink,seed:20,screens:[screen]).drag(from:CGRect(x:80,y:80,width:50,height:30),to:CGRect(x:700,y:500,width:60,height:30),within:screen)
results["drag_posts_held_path"] = dragSink.events.contains { $0["type"] as? UInt32 == CGEventType.leftMouseDragged.rawValue }
    && dragSink.events.last?["type"] as? UInt32 == CGEventType.leftMouseUp.rawValue
let cancelSink = TestSink()
cancelSink.cancelOnDrag = true
do {
    try Actions(sink:cancelSink,seed:30,screens:[screen],checkAllowed:{ if cancelSink.cancelled { throw ActionError.cancelled } })
        .drag(from:CGRect(x:80,y:80,width:50,height:30),to:CGRect(x:700,y:500,width:60,height:30),within:screen)
    results["cancelled_drag_releases"] = false
} catch {
    results["cancelled_drag_releases"] = cancelSink.events.last?["type"] as? UInt32 == CGEventType.leftMouseUp.rawValue
}
let keySink = TestSink()
try Actions(sink:keySink,seed:1,screens:[screen]).key("cmd+shift+s")
results["combined_modifiers"] = keySink.events.contains { $0["key"] as? Int == 1 && $0["down"] as? Bool == true
    && $0["flags"] as? UInt64 == CGEventFlags([.maskCommand,.maskShift]).rawValue }
let failingSink = TestSink()
failingSink.failKeyDown = 1
do { try Actions(sink:failingSink).key("cmd+shift+s"); results["failed_keys_release"] = false }
catch { results["failed_keys_release"] = [1,56,55].allSatisfy { key in failingSink.events.contains { $0["key"] as? Int == key && $0["down"] as? Bool == false } } }
do { _ = try Actions.shortcut("cmd+cmd+s"); results["invalid_shortcut_rejected"] = false }
catch { results["invalid_shortcut_rejected"] = true }

let bounds = WindowRect(x:0,y:0,w:800,h:600)
let control = ObservedControl(id:"test",source:"accessibility",role:"AXButton",label:"Synthetic",value:nil,enabled:true,focused:false,
    bounds:WindowRect(x:10,y:10,w:50,h:30),path:[0],actions:[])
func observation(pid: Int32 = 1, fingerprint: String = "same", controls: [ObservedControl] = [control]) -> DesktopObservation {
    DesktopObservation(bundle_id:"test.fixture",pid:pid,window_id:1,title:"Fixture",bounds:bounds,fingerprint:fingerprint,controls:controls,captured_at_ms:1,screenshot_base64:nil,accessibility_available:true,accessibility_truncated:false,coverage:"visible_window_only")
}
results["valid_binding"] = (try? validateBinding(saved:observation(),current:observation(),target:"test")) != nil
results["caret_or_focus_render_changes_do_not_invalidate_ax_target"] = (try? validateBinding(saved:observation(),current:observation(fingerprint:"changed"),target:"test")) != nil
results["replaced_process_rejected"] = (try? validateBinding(saved:observation(),current:observation(pid:2),target:"test")) == nil
results["missing_target_rejected"] = (try? validateBinding(saved:observation(),current:observation(),target:"missing")) == nil
results["invalid_geometry_rejected"] = !usable(WindowRect(x:790,y:10,w:50,h:30),within:bounds)

let textField = ObservedControl(id:"field",source:"accessibility",role:"AXTextField",label:"Name",value:"before",enabled:true,focused:false,
    bounds:WindowRect(x:10,y:10,w:80,h:30),path:[1],actions:[])
let focusedField = ObservedControl(id:"new-field",source:"accessibility",role:"AXTextField",label:"Name",value:"before",enabled:true,focused:true,
    bounds:textField.bounds,path:[1],actions:[])
results["textfield_focus_ring_change_is_not_stale"] = (try? validateBinding(saved:observation(controls:[textField]),current:observation(fingerprint:"new-render",controls:[focusedField]),target:"field")) != nil
let changedValue = ObservedControl(id:"field",source:"accessibility",role:"AXTextField",label:"Name",value:"edited",enabled:true,focused:true,
    bounds:textField.bounds,path:[1],actions:[])
results["changed_target_value_rejected"] = (try? validateBinding(saved:observation(controls:[textField]),current:observation(controls:[changedValue]),target:"field")) == nil
let moved = ObservedControl(id:"test",source:"accessibility",role:"AXButton",label:"Synthetic",value:nil,enabled:true,focused:false,
    bounds:WindowRect(x:110,y:10,w:50,h:30),path:[0],actions:[])
results["moved_target_rejected"] = (try? validateBinding(saved:observation(),current:observation(controls:[moved]),target:"test")) == nil
results["ambiguous_ax_path_rejected"] = (try? validateBinding(saved:observation(),current:observation(controls:[control,control]),target:"test")) == nil
let ocr = ObservedControl(id:"ocr-old",source:"ocr",role:"text",label:"Label",value:nil,enabled:true,focused:false,
    bounds:control.bounds,path:nil,actions:[],confidence:0.99,visual_fingerprint:"crop")
let ocrLater = ObservedControl(id:"ocr-new",source:"ocr",role:"text",label:"Label",value:nil,enabled:true,focused:false,
    bounds:control.bounds,path:nil,actions:[],confidence:0.95,visual_fingerprint:"crop")
let ocrChanged = ObservedControl(id:"ocr-new",source:"ocr",role:"text",label:"Label",value:nil,enabled:true,focused:false,
    bounds:control.bounds,path:nil,actions:[],visual_fingerprint:"changed-crop")
results["ocr_confidence_jitter_and_other_window_pixels_ignored"] = (try? validateBinding(saved:observation(controls:[ocr]),current:observation(fingerprint:"changed",controls:[ocrLater]),target:"ocr-old")) != nil
results["ocr_local_pixel_change_rejected"] = (try? validateBinding(saved:observation(controls:[ocr]),current:observation(controls:[ocrChanged]),target:"ocr-old")) == nil
let readbackResult = try actionResult(observation:nil,verification:"timeout",inputReadback:"passed",activation:"not_applicable",observationError:"deadline_exceeded")
results["readback_survives_expected_text_timeout"] = readbackResult["input_readback"] as? String == "passed" && readbackResult["verification"] as? String == "timeout"
let samePackageOverlay = URL(fileURLWithPath:"/synthetic/bin/wechat_overlay")
results["only_package_passive_overlay_is_trusted"] = isPackagePassiveOverlay(executable:samePackageOverlay,helper:URL(fileURLWithPath:"/synthetic/bin/desktop_helper")) && !isPackagePassiveOverlay(executable:URL(fileURLWithPath:"/other/bin/wechat_overlay"),helper:URL(fileURLWithPath:"/synthetic/bin/desktop_helper"))
results["passive_overlay_does_not_occlude_target"] = !blocksAction(OcclusionCandidate(bounds:bounds,alpha:1,passive:true),regions:[control.bounds])
results["unrelated_window_area_does_not_occlude_target"] = !blocksAction(OcclusionCandidate(bounds:WindowRect(x:300,y:0,w:200,h:200),alpha:1,passive:false),regions:[control.bounds])
results["interactive_window_over_target_blocks"] = blocksAction(OcclusionCandidate(bounds:bounds,alpha:1,passive:false),regions:[control.bounds])
var now: TimeInterval = 0
let total = InteractionDeadline(seconds:30,clock:{now})
now = 12
let afterPrecheck = total.remaining
now = 25
let afterReadback = total.remaining
now = 31
results["deadline_is_shared_across_phases"] = afterPrecheck == 18 && afterReadback == 5 && total.remaining == 0
do { try total.check(); results["expired_budget_stops_next_phase"] = false }
catch { results["expired_budget_stops_next_phase"] = true }
now = 0
let activationDeadline = InteractionDeadline(seconds:3,clock:{now})
do {
    try awaitActivation(deadline:activationDeadline,check:{false},wait:{now += 1})
    results["activation_request_without_foreground_fails"] = false
} catch { results["activation_request_without_foreground_fails"] = true }
now = 0
var probes = 0
try awaitActivation(deadline:InteractionDeadline(seconds:3,clock:{now}),check:{probes += 1;return probes >= 2},wait:{now += 1})
results["activation_waits_for_actual_foreground"] = probes == 2
print(String(decoding:try JSONSerialization.data(withJSONObject:results,options:.sortedKeys),as:UTF8.self))
if results.values.contains(false) { exit(1) }
