// Isolated manual acceptance fixture. No network, files, clipboard or business data.
import AppKit

final class FixtureDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    let status = NSTextField(labelWithString:"Fixture ready")
    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect:NSRect(x:180,y:200,width:440,height:260),styleMask:[.titled,.closable],backing:.buffered,defer:false)
        window.title = "Desktop interaction acceptance fixture"
        let button = NSButton(title:"Run fixture action",target:self,action:#selector(runFixture))
        button.frame = NSRect(x:100,y:100,width:230,height:40)
        status.frame = NSRect(x:100,y:170,width:240,height:30)
        window.contentView!.addSubview(status)
        window.contentView!.addSubview(button)
        window.initialFirstResponder = button
        window.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps:true)
        print("fixture_ready")
        fflush(stdout)
    }
    @objc func runFixture() { status.stringValue = "Fixture completed" }
}
let delegate = FixtureDelegate()
let app = NSApplication.shared
app.setActivationPolicy(.regular)
app.delegate = delegate
app.run()
