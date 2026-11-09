import Foundation

private func frText(_ line:[String:Any])->String { (line["text"] as? String ?? "").trimmingCharacters(in:.whitespacesAndNewlines) }
private func frY(_ line:[String:Any])->Double { (line["y"] as? Double ?? 0) + (line["h"] as? Double ?? 0)/2 }
private func frX(_ line:[String:Any])->Double { (line["x"] as? Double ?? 0) + (line["w"] as? Double ?? 0)/2 }

/// Normalize OCR status labels without relying on the easily confused first character.
func friendRequestStatus(_ text:String) -> String? {
 let text=String(text.filter{!$0.isWhitespace})
 if (3...5).contains(text.count),text.hasSuffix("验证") {return "等待验证"}
 if text.contains("添加") {return "已添加"}
 if text.contains("过期") {return "已过期"}
 return nil
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
  friendRequestStatus(row["status"] as? String ?? "")=="等待验证" else{return nil}
 return row
}

/// Input coordinates come from Vision's normalized boxes multiplied by the captured window size.
public func parseFriendRequestRows(_ lines:[[String:Any]])->[[String:Any]] {
 let sorted=lines.sorted{frY($0)<frY($1)}
 guard let header=sorted.first(where:{frText($0).contains("新的朋友")}) else{return []}
 let lower=frY(header)
 let upper=sorted.filter{frY($0)>lower && frText($0).contains("公众号")}.map{frY($0)}.min() ?? Double.infinity
 let body=sorted.filter{frY($0)>lower && frY($0)<upper}
 let statuses=body.filter{friendRequestStatus(frText($0)) != nil}
 var rows:[[String:Any]]=[]
 for (i,status) in statuses.enumerated() {
  let tolerance=max(1,status["h"] as? Double ?? 0)
  guard let name=body.filter({abs(frY($0)-frY(status))<tolerance && frX($0)<(status["x"] as? Double ?? 0)}).first else {continue}
  let end=i+1<statuses.count ? frY(statuses[i+1])-tolerance : upper
  let message=body.filter{frY($0)>frY(status)+tolerance && frY($0)<end && ($0["x"] as? Double ?? 0) >= (name["x"] as? Double ?? 0)-tolerance}.map{frText($0)}.joined(separator:"\n")
  rows.append(["row_y":frY(name),"name":frText(name),"status":friendRequestStatus(frText(status))!,"message":message,"name_x":frX(name)])
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
 var result:[String:Any]=["source":source,"message":message.joined(separator:"\n"),"verify_button":NSNull()]
 if let button=sorted.first(where:{frText($0)=="前往验证"}) {result["verify_button"]=["x":frX(button),"y":frY(button)]}
 return result
}

#if BOT_WORKER
import CoreGraphics
import AppKit

private func friendLines(_ window:[String:Any]) throws -> [[String:Any]] {
 let normalized=try normalizedFrame(windowImage(window),window:window)
 return try recognizeContactLines(image:normalized.image,timeout:2)
}
private func friendClick(_ line:[String:Any],window:[String:Any]) throws {
 let x=frX(line), y=frY(line)
 guard x.isFinite,y.isFinite,x>0,y>0,x<(window["width"] as? Double ?? 0),y<(window["height"] as? Double ?? 0) else {throw Failure.invalid("friend_request_coordinates")}
 try act {try botActions.click(CGRect(x:(window["x"] as! Double)+x-1,y:(window["y"] as! Double)+y-1,width:2,height:2))}
}
private func friendListLines(_ window:[String:Any]) throws -> [[String:Any]] {
 let spec=try currentLayout().contactsSpec()
 return try friendLines(window).filter{spec.list.contains(WindowPoint(x:frX($0),y:frY($0)))}
}
private func friendDetailLines(_ window:[String:Any]) throws -> [[String:Any]] {
 let spec=try currentLayout().contactsSpec()
 return try friendLines(window).filter{spec.profile.contains(WindowPoint(x:frX($0),y:frY($0)))}
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
  if parseFriendRequestRows(lines).isEmpty {
   try friendClick(header,window:window);pauseUI(0.25);lines=try friendListLines(window)
  }
  return ["rows":parseFriendRequestRows(lines)]
 }
 if operation=="friend_request_detail" {
  guard params["row_y"] as? Double != nil, let name=params["name"] as? String else{throw Failure.invalid("friend_request_row_missing")}
  let lines=try friendListLines(window)
  guard let row=uniqueFriendRequestRow(parseFriendRequestRows(lines),name),
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
  var mustEscape=true
  defer {if mustEscape {try? key(53)}}
  let before=try friendDetailLines(window)
  guard before.contains(where:{sameFriendName(frText($0),name)}) else{throw Failure.invalid("friend_request_detail_mismatch")}
  guard let verify=before.first(where:{frText($0)=="前往验证"}) else{throw Failure.invalid("friend_request_verify_button_missing")}
  let directory=URL(fileURLWithPath:path)
  try friendPrivateDirectory(directory.deletingLastPathComponent().deletingLastPathComponent())
  try friendPrivateDirectory(directory.deletingLastPathComponent())
  try friendPrivateDirectory(directory)
  try friendClick(verify,window:window)
  let deadline=Date().addingTimeInterval(3)
  var screens:[([String:Any],[[String:Any]])]=[]
  repeat {
   pauseUI(0.15)
   screens=[]
   var windows=friendWindows(window)
   if !windows.contains(where:{($0["window_id"] as? UInt32)==(window["window_id"] as? UInt32)}){windows.append(window)}
   for (i,w) in windows.enumerated() {
    let image=try windowImage(w)
    try writePNG(image,path:directory.appendingPathComponent("window-\(i).png"))
    let lines=try recognizeContactLines(image:image,timeout:1)
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
   if screens.contains(where:{$0.1.contains(where:{frText($0)=="取消"})}){break}
  } while Date()<deadline
  let candidates=screens.filter{$0.1.contains(where:{frText($0)=="取消"})}
  guard candidates.count==1,let (dialog,lines)=candidates.first else{throw Failure.invalid("friend_request_dialog_unknown")}
  let allowed=["取消","确定","完成","确认","添加朋友","朋友验证","备注","标签","朋友权限","设置备注和标签","仅聊天","聊天、朋友圈、微信运动等","朋友","添加到通讯录"]
  guard lines.allSatisfy({allowed.contains(frText($0)) || frText($0)==name}),lines.contains(where:{["添加朋友","朋友验证","朋友权限"].contains(frText($0))}) else{throw Failure.invalid("friend_request_dialog_unknown")}
  if mode=="dry_run" {
   guard let cancel=lines.first(where:{frText($0)=="取消"}) else{throw Failure.invalid("friend_request_dialog_unknown")}
   try friendClick(cancel,window:dialog)
  } else {
   // Unobserved forms require review unless every control belongs to this small known vocabulary.
   let buttons=lines.filter{["确定","完成","确认"].contains(frText($0))}
   guard buttons.count==1 else{throw Failure.invalid("friend_request_dialog_unknown")}
   try friendClick(buttons[0],window:dialog)
  }
  pauseUI(0.35)
  try screenGate(window)
  let rows=parseFriendRequestRows(try friendListLines(window)).filter{sameFriendName($0["name"] as? String ?? "",name)}
  let detail=parseFriendRequestDetail(try friendDetailLines(window))
  let verified:Bool
  if mode=="dry_run" {verified=rows.count==1 && friendRequestStatus(rows[0]["status"] as? String ?? "")=="等待验证" && detail["verify_button"] is [String:Double]}
  else {verified=rows.count==1 && friendRequestStatus(rows[0]["status"] as? String ?? "")=="已添加"}
  guard verified else{throw Failure.invalid("friend_request_verification_mismatch")}
  mustEscape = false
  return ["verified":true]
 }
 throw Failure.invalid("friend_request_operation_unknown")
}
#endif
