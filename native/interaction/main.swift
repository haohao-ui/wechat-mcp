import Foundation
import AppKit
import CoreGraphics

while let line = readLine() {
  autoreleasepool {
   do {
    let input = Data(line.utf8)
    let command = try JSONDecoder().decode(DesktopCommand.self,from:input)
    let result: Any
    switch command.operation {
    case "apps":
        result = NSWorkspace.shared.runningApplications.compactMap { app -> [String:Any]? in
            guard let bundle = app.bundleIdentifier, app.activationPolicy == .regular else { return nil }
            return ["bundle_id":bundle,"pid":app.processIdentifier,"name":app.localizedName ?? "","active":app.isActive]
        }
    case "windows":
        guard let bundle = command.bundle_id else { throw InteractionFailure("bundle_id_required") }
        let app = try runningApp(bundle)
        let rows = CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
        result = try rows.filter { ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == app.processIdentifier && ($0[kCGWindowLayer as String] as? Int) == 0 }.map { row -> [String:Any] in
            let bounds = try boundsOf(row)
            return ["window_id":row[kCGWindowNumber as String] ?? 0,"title":row[kCGWindowName as String] ?? "","bounds":["x":bounds.x,"y":bounds.y,"w":bounds.w,"h":bounds.h]]
        }
    case "observe":
        guard let bundle = command.bundle_id, !bundle.isEmpty else { throw InteractionFailure("bundle_id_required") }
        result = try JSONSerialization.jsonObject(with:JSONEncoder().encode(observe(bundle:bundle,windowID:command.window_id,screenshot:command.include_screenshot ?? false)))
    case "act": result = try act(command)
    default: throw InteractionFailure("unsupported_operation")
    }
    let data = try JSONSerialization.data(withJSONObject:["ok":true,"result":result],options:[.sortedKeys])
    print(String(decoding:data,as:UTF8.self))
    fflush(stdout)
} catch {
    let data = try! JSONSerialization.data(withJSONObject:["ok":false,"error":String(describing:error)],options:[.sortedKeys])
    print(String(decoding:data,as:UTF8.self))
    fflush(stdout)
   }
  }
}
