import Foundation

private func frText(_ line:[String:Any])->String { (line["text"] as? String ?? "").trimmingCharacters(in:.whitespacesAndNewlines) }
private func frY(_ line:[String:Any])->Double { (line["y"] as? Double ?? 0) + (line["h"] as? Double ?? 0)/2 }
private func frX(_ line:[String:Any])->Double { (line["x"] as? Double ?? 0) + (line["w"] as? Double ?? 0)/2 }

// Closed UI vocabulary from docs/observations/2026-11-06-wechat-4.1.15-actions.md §9.
// Observe new UI wording on the real client before adding entries; never add OCR typo aliases.
enum FriendWordKind { case status, source, button, dialog }
private let friendVocabulary: [FriendWordKind:[String]] = [
 .status:["等待验证","已添加","已过期"],
 .source:["通过搜索手机号添加","通过搜索微信号添加","通过搜索QQ号添加","通过扫一扫添加","通过群聊添加","通过名片分享添加"],
 .button:["前往验证","取消","确定","完成","确认"],
 // 「通过朋友验证」 dialog, observed 2026-11-09 (§9). Field contents (remark, tag placeholder) are exempt by position.
 .dialog:["通过朋友验证","备注","标签","朋友权限","聊天、朋友圈、微信运动等","仅聊天","朋友圈和状态","不让他（她）看","不看他（她）","取消","确定"]
]
struct FriendWordMatch {
 let word:String?
 let best:Double
 let second:Double
 var diagnostic:String { "best=\(String(format:"%.3f",best)),second=\(String(format:"%.3f",second))" }
}
private func friendDistance(_ a:[Character],_ b:[Character])->Int {
 var previous=Array(0...b.count)
 for (i,x) in a.enumerated() {
  var row=[i+1]
  for (j,y) in b.enumerated() {row.append(min(row[j]+1,previous[j+1]+1,previous[j]+(x==y ? 0:1)))}
  previous=row
 }
 return previous[b.count]
}
/// Unicode compatibility form (full/half width) only; never per-character OCR corrections.
private func friendNormalized(_ text:String)->[Character] {Array(text.precomposedStringWithCompatibilityMapping.filter{!$0.isWhitespace})}
func friendWordMatch(_ text:String,_ kind:FriendWordKind)->FriendWordMatch {
 let text=friendNormalized(text)
 let scores=(friendVocabulary[kind] ?? []).map { word -> (String,Double,Int) in
  let chars=friendNormalized(word), distance=friendDistance(text,chars)
  return (word,1-Double(distance)/Double(max(1,max(chars.count,text.count))),distance)
 }.sorted{$0.1>$1.1}
 let best=scores[0], second=scores[1].1
 let accepted=best.1>=0.75 && best.1-second>=0.15-1e-9 && best.2<=1
 return FriendWordMatch(word:accepted ? best.0:nil,best:best.1,second:second)
}
func friendWordConsensus(_ two:String,_ three:String,_ kind:FriendWordKind)->FriendWordMatch {
 let a=friendWordMatch(two,kind), b=friendWordMatch(three,kind)
 return FriendWordMatch(word:a.word != nil && a.word==b.word ? a.word:nil,best:min(a.best,b.best),second:max(a.second,b.second))
}
func friendRequestStatus(_ text:String)->String? {friendWordMatch(text,.status).word}
private func friendCanonical(_ line:[String:Any],_ kind:FriendWordKind)->String? {
 if line["fr_checked"] as? Bool == true {return line["fr_word"] as? String}
 return friendWordMatch(frText(line),kind).word
}
private func friendUnknown(_ item:String,_ line:[String:Any]?)->String {
 "unknown_\(item) \(line?["fr_scores"] as? String ?? "best=0.000,second=0.000")"
}

func sameFriendName(_ a:String,_ b:String) -> Bool {
 let a=Array(a.filter{!$0.isWhitespace}),b=Array(b.filter{!$0.isWhitespace})
 if a==b {return true}
 guard min(a.count,b.count)>=3,abs(a.count-b.count)<=1 else{return false}
 // A single insertion, deletion, or substitution; compare grapheme clusters.
 var i=0,j=0,edits=0
 while i<a.count && j<b.count {
  if a[i]==b[j] {i+=1;j+=1;continue}
  edits+=1
  if edits>1 {return false}
  if a.count>=b.count {i+=1}
  if b.count>=a.count {j+=1}
 }
 return edits+(a.count-i)+(b.count-j)<=1
}

/// Count every similar nickname before checking status, including already handled rows.
func uniqueFriendRequestRow(_ rows:[[String:Any]],_ name:String) -> [String:Any]? {
 let matching=rows.filter{sameFriendName($0["name"] as? String ?? "",name)}
 guard matching.count==1,let row=matching.first,
  row["status"] as? String == "等待验证" else{return nil}
 return row
}

/// Input coordinates come from Vision's normalized boxes multiplied by the captured window size.
public func parseFriendRequestRows(_ lines:[[String:Any]],requireConsensus:Bool=false)->[[String:Any]] {
 let sorted=lines.sorted{frY($0)<frY($1)}
 guard let header=sorted.first(where:{frText($0).contains("新的朋友")}) else{return []}
 let lower=frY(header)
 let upper=sorted.filter{frY($0)>lower && frText($0).contains("公众号")}.map{frY($0)}.min() ?? Double.infinity
 let body=sorted.filter{frY($0)>lower && frY($0)<upper}
 let statuses=body.filter{$0["fr_kind"] as? String == "status" || (!requireConsensus && friendCanonical($0,.status) != nil)}
 var rows:[[String:Any]]=[]
 for (i,status) in statuses.enumerated() {
  let tolerance=max(1,status["h"] as? Double ?? 0)
  guard let name=body.filter({abs(frY($0)-frY(status))<tolerance && frX($0)<(status["x"] as? Double ?? 0)}).first else {continue}
  let end=i+1<statuses.count ? frY(statuses[i+1])-tolerance : upper
  let message=body.filter{frY($0)>frY(status)+tolerance && frY($0)<end && ($0["x"] as? Double ?? 0) >= (name["x"] as? Double ?? 0)-tolerance}.map{frText($0)}.joined(separator:"\n")
  var row:[String:Any]=["row_y":frY(name),"name":frText(name),"status":friendCanonical(status,.status) ?? "unknown","message":message,"name_x":frX(name)]
  if row["status"] as? String == "unknown" {row["error"]=friendUnknown("status",status)}
  rows.append(row)
 }
 return rows
}

public func parseFriendRequestDetail(_ lines:[[String:Any]])->[String:Any] {
 let sorted=lines.sorted{frY($0)<frY($1)}
 var source="", message:[String]=[]
 let sourceLine=sorted.first{frText($0).hasPrefix("来源")}
 if let l=sourceLine {
  source=String(frText(l).dropFirst(2)).trimmingCharacters(in:CharacterSet(charactersIn:" ：:"))
  if source.isEmpty, let value=sorted.first(where:{frX($0)>frX(l) && abs(frY($0)-frY(l)) <= max(1,l["h"] as? Double ?? 0)}) {source=frText(value)}
 }
 if let first=sorted.first {
  let end=sorted.first(where:{frText($0)=="回复"}).map{frY($0)} ?? sourceLine.map{frY($0)} ?? frY(first)
  message=sorted.filter{frY($0)>frY(first)+max(1,first["h"] as? Double ?? 0) && frY($0)<end}.map{frText($0)}
 }
 let sourceResult: String?
 if let line=sourceLine, line["fr_checked"] as? Bool == true {sourceResult=line["fr_word"] as? String}
 else {sourceResult=friendWordMatch(source,.source).word}
 var result:[String:Any]=["source":sourceResult ?? "","message":message.joined(separator:"\n"),"verify_button":NSNull()]
 if sourceResult == nil {result["error"]=friendUnknown("source",sourceLine)}
 if let button=sorted.first(where:{friendCanonical($0,.button)=="前往验证"}) {result["verify_button"]=["x":frX(button),"y":frY(button)]}
 else if result["error"] == nil {result["error"]=friendUnknown("button",sorted.first{$0["fr_kind"] as? String == "button"})}
 if let unknown=sorted.first(where:{$0["fr_kind"] as? String == "button" && friendCanonical($0,.button)==nil}),result["error"] == nil {result["error"]=friendUnknown("button",unknown);result["verify_button"]=NSNull()}
 if result["error"] != nil {result["verify_button"]=NSNull()}
 return result
}

/// Validates the 「通过朋友验证」 dialog. Every line must be a consensus vocabulary word except the
/// remark field (between 备注 and 标签) and the tag field (between 标签 and 朋友权限).
public func classifyFriendDialog(_ lines:[[String:Any]])->[String:Any] {
 let sorted=lines.sorted{frY($0)<frY($1)}
 func word(_ l:[String:Any])->String? {friendCanonical(l,.dialog)}
 func first(_ w:String)->[String:Any]? {sorted.first{word($0)==w}}
 guard first("通过朋友验证") != nil,let remark=first("备注"),let tag=first("标签"),let permission=first("朋友权限"),
  frY(remark)<frY(tag),frY(tag)<frY(permission) else {return ["error":friendUnknown("dialog",sorted.first{word($0)==nil})]}
 let cancels=sorted.filter{word($0)=="取消"},confirms=sorted.filter{word($0)=="确定"}
 guard cancels.count==1,confirms.count==1 else {return ["error":friendUnknown("dialog_button",nil)]}
 let fields={(l:[String:Any])->Bool in frY(l)>frY(remark) && frY(l)<frY(tag) || frY(l)>frY(tag) && frY(l)<frY(permission)}
 if let unknown=sorted.first(where:{word($0)==nil && !fields($0)}) {return ["error":friendUnknown("dialog",unknown)]}
 return ["cancel":cancels[0],"confirm":confirms[0]]
}

#if BOT_WORKER
import CoreGraphics
import AppKit

private func friendClick(_ line:[String:Any],window:[String:Any]) throws {
 let x=frX(line), y=frY(line)
 guard x.isFinite,y.isFinite,x>0,y>0,x<(window["width"] as? Double ?? 0),y<(window["height"] as? Double ?? 0) else {throw Failure.invalid("friend_request_coordinates")}
 try act {try botActions.click(CGRect(x:(window["x"] as! Double)+x-1,y:(window["y"] as! Double)+y-1,width:2,height:2))}
}
/// Crop in normalized window points, interpolate before OCR, then undo scale and offset.
private func friendRegionLines(_ image:CGImage,_ rect:CGRect,_ scale:Int) throws -> [[String:Any]] {
 let rect=rect.integral.intersection(CGRect(x:0,y:0,width:image.width,height:image.height))
 guard !rect.isNull,rect.width>0,rect.height>0,let crop=image.cropping(to:rect),
  let context=CGContext(data:nil,width:crop.width*scale,height:crop.height*scale,bitsPerComponent:8,bytesPerRow:0,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else {throw Failure.invalid("friend_request_crop_invalid")}
 context.interpolationQuality = .high
 context.draw(crop,in:CGRect(x:0,y:0,width:crop.width*scale,height:crop.height*scale))
 guard let enlarged=context.makeImage() else {throw Failure.invalid("friend_request_resize_failed")}
 return try recognizeContactLines(image:enlarged,timeout:2).map { line in
  var result=line
  for key in ["x","w"] {result[key]=(line[key] as? Double ?? 0)/Double(scale)+(key=="x" ? Double(rect.minX):0)}
  for key in ["y","h"] {result[key]=(line[key] as? Double ?? 0)/Double(scale)+(key=="y" ? Double(rect.minY):0)}
  return result
 }
}
private func friendRefine(_ image:CGImage,_ line:[String:Any],_ kind:FriendWordKind,_ rect:CGRect)->[String:Any] {
 var result=line
 var reads:[String]=[], located:[[String:Any]]=[]
 for scale in [2,3] {
  let lines=(try? friendRegionLines(image,rect,scale)) ?? []
  var text=lines.sorted{frX($0)<frX($1)}.map{frText($0)}.joined()
  if kind == .source, text.hasPrefix("来源") {text=String(text.dropFirst(2)).trimmingCharacters(in:CharacterSet(charactersIn:" ：:"))}
  reads.append(text)
  if scale==3 {located=lines}
 }
 let match=friendWordConsensus(reads[0],reads[1],kind)
 result["fr_checked"]=true;result["fr_word"]=match.word;result["fr_scores"]=match.diagnostic
 result["fr_kind"]=kind == .status ? "status":kind == .source ? "source":kind == .dialog ? "dialog":"button"
 // Use the high-resolution text geometry, never the center of the crop, for clicking.
 if kind == .button,match.word != nil,!located.isEmpty {
  let left=located.map{$0["x"] as? Double ?? 0}.min()!,top=located.map{$0["y"] as? Double ?? 0}.min()!
  let right=located.map{($0["x"] as? Double ?? 0)+($0["w"] as? Double ?? 0)}.max()!,bottom=located.map{($0["y"] as? Double ?? 0)+($0["h"] as? Double ?? 0)}.max()!
  result["x"]=left;result["y"]=top;result["w"]=right-left;result["h"]=bottom-top
 }
 return result
}
private func friendListLines(_ window:[String:Any]) throws -> [[String:Any]] {
 let spec=try currentLayout().contactsSpec()
 let image=try normalizedFrame(windowImage(window),window:window).image
 let lines=try recognizeContactLines(image:image,timeout:2).filter{spec.list.contains(WindowPoint(x:frX($0),y:frY($0)))}
 guard let header=lines.first(where:{frText($0).contains("新的朋友")}) else{return lines}
 let upper=lines.filter{frY($0)>frY(header) && frText($0).contains("公众号")}.map{frY($0)}.min() ?? spec.list.y+spec.list.h
 let right=spec.list.x+spec.list.w*0.55
 let region=CGRect(x:right,y:frY(header)+max(12,header["h"] as? Double ?? 0),width:spec.list.x+spec.list.w-right,height:max(1,upper-frY(header)-max(12,header["h"] as? Double ?? 0)))
 let two=(try? friendRegionLines(image,region,2)) ?? []
 let three=(try? friendRegionLines(image,region,3)) ?? []
 var statuses:[[String:Any]]=[]
 // Union both passes: a line missed by either pass is explicitly unknown.
 for line in three + two.filter({a in !three.contains{abs(frY(a)-frY($0))<max(6,a["h"] as? Double ?? 0)}}) {
  let tolerance=max(6,line["h"] as? Double ?? 0)
  let peers=two.filter{abs(frY($0)-frY(line))<tolerance}
  let counterparts=three.filter{abs(frY($0)-frY(line))<tolerance}
  let match=friendWordConsensus(peers.count==1 ? frText(peers[0]):"",counterparts.count==1 ? frText(counterparts[0]):"",.status)
  var status=line;status["fr_checked"]=true;status["fr_word"]=match.word;status["fr_scores"]=match.diagnostic;status["fr_kind"]="status"
  statuses.append(status)
 }
 if statuses.isEmpty {
  // An expanded group with unreadable right-side text is not an empty/collapsed group.
  // Keep possible rows non-actionable instead of clicking the header again.
  for name in lines.filter({frY($0)>frY(header)+12 && frY($0)<upper && frX($0)<right && frX($0)>spec.list.x+spec.list.w*0.25}) {
   statuses.append(["text":"","x":right,"y":name["y"] ?? 0,"w":spec.list.x+spec.list.w-right,"h":name["h"] ?? 12,"fr_checked":true,"fr_kind":"status","fr_scores":"best=0.000,second=0.000"])
  }
 }
 return lines.filter{!(frY($0)>frY(header) && frY($0)<upper && frX($0)>right)}+statuses
}
private func friendDetailLines(_ window:[String:Any]) throws -> [[String:Any]] {
 let spec=try currentLayout().contactsSpec()
 let image=try normalizedFrame(windowImage(window),window:window).image
 let lines=try recognizeContactLines(image:image,timeout:2).filter{spec.profile.contains(WindowPoint(x:frX($0),y:frY($0)))}
 let source=lines.first{frText($0).hasPrefix("来源")}
 return lines.map {line in
  let h=max(12,line["h"] as? Double ?? 0)
  if let source=source,frY(line)==frY(source),frX(line)==frX(source) {
   return friendRefine(image,line,.source,CGRect(x:spec.profile.x,y:frY(line)-h,width:spec.profile.w,height:h*2))
  }
  if (source.map{frY(line)>frY($0)+h} ?? false) || friendWordMatch(frText(line),.button).best>=0.5 {
   return friendRefine(image,line,.button,CGRect(x:(line["x"] as? Double ?? 0)-h/2,y:frY(line)-h,width:(line["w"] as? Double ?? 0)+h,height:h*2))
  }
  return line
 }
}
private func friendDialogLines(_ image:CGImage)->[[String:Any]] {
 let lines=(try? recognizeContactLines(image:image,timeout:2)) ?? []
 return lines.map {line in
  let h=max(12,line["h"] as? Double ?? 0)
  return friendRefine(image,line,.dialog,CGRect(x:(line["x"] as? Double ?? 0)-h/2,y:frY(line)-h,width:(line["w"] as? Double ?? 0)+h,height:h*2))
 }
}
/// Closes windows that appeared after 前往验证: unique consensus 取消, else Esc; then requires them gone.
private func friendDismiss(_ main:[String:Any],opened before:Set<UInt32>) throws {
 func extra()->[[String:Any]] {friendWindows(main).filter{!before.contains($0["window_id"] as? UInt32 ?? 0)}}
 for w in extra() {
  guard let image=try? windowImage(w) else {continue}
  let sx=(w["width"] as! Double)/Double(image.width),sy=(w["height"] as! Double)/Double(image.height)
  let cancels=friendDialogLines(image).filter{friendCanonical($0,.dialog)=="取消"}.map {l -> [String:Any] in var l=l;for k in ["x","w"]{l[k]=(l[k] as? Double ?? 0)*sx};for k in ["y","h"]{l[k]=(l[k] as? Double ?? 0)*sy};return l}
  if cancels.count==1 {try? friendClick(cancels[0],window:w)} else {try? key(53)}
  pauseUI(0.35)
 }
 if !extra().isEmpty {try? key(53);pauseUI(0.35)}
 guard extra().isEmpty else {throw Failure.invalid("friend_request_dialog_left_open")}
}
private func friendWindows(_ main:[String:Any])->[[String:Any]] {
 let pid=main["pid"] as? Int ?? 0
 let raw=CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
 return raw.compactMap { r in
  guard (r[kCGWindowOwnerPID as String] as? Int)==pid,
    (r[kCGWindowLayer as String] as? Int)==0,
    let id=r[kCGWindowNumber as String] as? UInt32,
    let b=r[kCGWindowBounds as String] as? [String:Any],let rect=CGRect(dictionaryRepresentation:b as CFDictionary),rect.width>0,rect.height>0 else{return nil}
  return ["pid":pid,"window_id":id,"x":Double(rect.minX),"y":Double(rect.minY),"width":Double(rect.width),"height":Double(rect.height)]
 }
}
private func friendPrivateDirectory(_ url:URL) throws {
 try FileManager.default.createDirectory(at:url,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
 try FileManager.default.setAttributes([.posixPermissions:0o700],ofItemAtPath:url.path)
}

func executeFriendRequestOperation(operation:String,params:[String:Any],manage:Bool) throws -> [String:Any] {
 guard desktopLeaseFD >= 0 || operation=="friend_request_badge" else {throw Failure.invalid("friend_request_lease_required")}
 let (window,_)=try readyWindow(manage:manage)
 try screenGate(window)
 if operation=="friend_request_badge" {
  let layout=try currentLayout(), spec=try currentLayout().contactsSpec()
  let image=try normalizedFrame(windowImage(window),window:window).image
  let pixels=try canonicalBytes(image)
  // Region derives from the calibrated navigation controls, with the same red predicate as session badges.
  let radius=abs(spec.navContacts.y-spec.navChats.y)/2
  var count=0
  for y in max(0,Int(spec.navContacts.y-radius))..<min(image.height,Int(spec.navContacts.y)) {
   for x in max(0,Int(spec.navContacts.x))..<min(image.width,Int(try layoutRegion(layout,"session_list").minX)) {
    let i=(y*image.width+x)*4
    if isBadgeRed(Int(pixels[i]),Int(pixels[i+1]),Int(pixels[i+2])){count+=1}
   }
  }
  return ["red_pixels":count]
 }
 if operation=="friend_request_scan" {
  var lines=try friendListLines(window)
  guard let header=lines.first(where:{frText($0).contains("新的朋友")}) else {throw Failure.invalid("friend_requests_header_missing")}
  if parseFriendRequestRows(lines,requireConsensus:true).isEmpty {
   try friendClick(header,window:window);pauseUI(0.25);lines=try friendListLines(window)
  }
  return ["rows":parseFriendRequestRows(lines,requireConsensus:true)]
 }
 if operation=="friend_request_detail" {
  guard params["row_y"] as? Double != nil, let name=params["name"] as? String else{throw Failure.invalid("friend_request_row_missing")}
  let lines=try friendListLines(window)
  let candidates=parseFriendRequestRows(lines,requireConsensus:true).filter{sameFriendName($0["name"] as? String ?? "",name)}
  if candidates.count==1,let row=candidates.first,row["status"] as? String == "unknown" {
   return ["source":"","message":row["message"] as? String ?? "","verify_button":NSNull(),"error":row["error"] as? String ?? friendUnknown("status",nil)]
  }
  guard let row=uniqueFriendRequestRow(candidates,name),
   let nameLine=lines.first(where:{sameFriendName(frText($0),name) && abs(frY($0)-(row["row_y"] as? Double ?? 0))<1}) else{throw Failure.invalid("friend_request_row_changed")}
  var nameTarget=nameLine
  nameTarget["w"]=min(nameLine["w"] as? Double ?? 0,nameLine["h"] as? Double ?? 0)
  try friendClick(nameTarget,window:window);pauseUI(0.35)
  let detail=try friendDetailLines(window)
  guard detail.contains(where:{sameFriendName(frText($0),name)}) else{throw Failure.invalid("friend_request_detail_mismatch")}
  return parseFriendRequestDetail(detail)
 }
 if operation=="friend_request_verify" {
  guard let mode=params["mode"] as? String,["dry_run","live"].contains(mode),let name=params["name"] as? String,
   let path=params["diagnostics_dir"] as? String else{throw Failure.invalid("friend_request_mode_invalid")}
  let before=try friendDetailLines(window)
  guard before.contains(where:{sameFriendName(frText($0),name)}) else{throw Failure.invalid("friend_request_detail_mismatch")}
  let checked=parseFriendRequestDetail(before)
  if let error=checked["error"] as? String {throw Failure.invalid(error)}
  guard let verify=before.first(where:{friendCanonical($0,.button)=="前往验证"}) else{throw Failure.invalid(friendUnknown("button",nil))}
  let directory=URL(fileURLWithPath:path)
  try friendPrivateDirectory(directory.deletingLastPathComponent().deletingLastPathComponent())
  try friendPrivateDirectory(directory.deletingLastPathComponent())
  try friendPrivateDirectory(directory)
  let opened=Set(friendWindows(window).compactMap{$0["window_id"] as? UInt32})
  try friendClick(verify,window:window)
  do {
   // The dialog is a separate window; wait for it to appear instead of a fixed delay.
   for _ in 0..<15 where friendWindows(window).allSatisfy({opened.contains($0["window_id"] as? UInt32 ?? 0)}) {pauseUI(0.15)}
   pauseUI(0.15)
   var screens:[([String:Any],[[String:Any]])]=[]
   var windows=friendWindows(window)
   if !windows.contains(where:{($0["window_id"] as? UInt32)==(window["window_id"] as? UInt32)}){windows.append(window)}
   for (i,w) in windows.enumerated() {
    let image=try windowImage(w)
    try writePNG(image,path:directory.appendingPathComponent("window-\(i).png"))
    let lines=friendDialogLines(image)
    let data=try JSONSerialization.data(withJSONObject:lines,options:[.prettyPrinted,.sortedKeys])
    let ocr=directory.appendingPathComponent("window-\(i).json")
    FileManager.default.createFile(atPath:ocr.path,contents:nil,attributes:[.posixPermissions:0o600])
    try data.write(to:ocr)
    try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:ocr.path)
    // Convert raw image pixels back to window points before clicking.
    let sx=(w["width"] as! Double)/Double(image.width),sy=(w["height"] as! Double)/Double(image.height)
    let scaled=lines.map {l -> [String:Any] in var l=l;for k in ["x","w"]{l[k]=(l[k] as? Double ?? 0)*sx};for k in ["y","h"]{l[k]=(l[k] as? Double ?? 0)*sy};return l}
    screens.append((w,scaled))
   }
   let candidates=screens.filter{!opened.contains($0.0["window_id"] as? UInt32 ?? 0)}.map{($0.0,classifyFriendDialog($0.1))}
   guard candidates.count==1,let (dialog,checked)=candidates.first else {throw Failure.invalid(friendUnknown("dialog",nil))}
   if let error=checked["error"] as? String {throw Failure.invalid(error)}
   try friendClick(checked[mode=="dry_run" ? "cancel":"confirm"] as! [String:Any],window:dialog)
   pauseUI(0.35)
   guard friendWindows(window).allSatisfy({opened.contains($0["window_id"] as? UInt32 ?? 0)}) else {throw Failure.invalid("friend_request_dialog_still_open")}
  } catch {
   // Never leave a verification dialog on screen: it blocks every later chat action.
   do {try friendDismiss(window,opened:opened)} catch let dismissal {throw Failure.invalid("\(error) + \(dismissal)")}
   throw error
  }
  pauseUI(0.35)
  try screenGate(window)
  let rows=parseFriendRequestRows(try friendListLines(window),requireConsensus:true).filter{sameFriendName($0["name"] as? String ?? "",name)}
  let detail=parseFriendRequestDetail(try friendDetailLines(window))
  guard rows.count==1 else{throw Failure.invalid(friendUnknown("status",nil))}
  if let row=rows.first, row["status"] as? String == "unknown" {throw Failure.invalid(row["error"] as? String ?? friendUnknown("status",nil))}
  if mode=="dry_run",let error=detail["error"] as? String {throw Failure.invalid(error)}
  let verified:Bool
  if mode=="dry_run" {verified=rows.count==1 && friendRequestStatus(rows[0]["status"] as? String ?? "")=="等待验证" && detail["verify_button"] is [String:Double]}
  else {verified=rows.count==1 && friendRequestStatus(rows[0]["status"] as? String ?? "")=="已添加"}
  guard verified else{throw Failure.invalid("friend_request_verification_mismatch")}
  return ["verified":true]
 }
 throw Failure.invalid("friend_request_operation_unknown")
}
#endif
