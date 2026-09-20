import AppKit

// An agent app: no Dock icon, no main window, no nib. The menu bar item is
// the whole interface until the library window arrives.
let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
