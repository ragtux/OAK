// macinput: the macOS half of macprobe.py. Real input through CGEvent and windows through the
// Accessibility API, so a scene can be played into Fleet (which has no in-app probe like WEEP's
// WEEP_KEYS). Needs Accessibility permission for the terminal that runs it (System Settings >
// Privacy & Security > Accessibility). Coordinates are global points, top-left origin.
//
//   macinput trusted                       1 when this process may post input, else 0
//   macinput front                         the frontmost app's pid
//   macinput windows <pid>...              JSON: on-screen windows of those pids (id, pid, bounds, layer, title)
//   macinput place <pid> <x> <y> <w> <h>   move and size the pid's main window (points)
//   macinput focus <pid>                   bring the app to the front and raise its main window
//   macinput key <chord>                   e.g. cmd-shift-f, escape, down, cmd-leftbracket
//   macinput type <text>                   unicode text, one key event per character
//   macinput move <x> <y>                  pointer
//   macinput down|up [left|right] <x> <y>  half a click
//   macinput click [left|right|double] <x> <y>
//   macinput wheel <x> <y> <lines>         positive scrolls up
//   macinput close <pid>                   AXPress the main window's close button

import AppKit
import ApplicationServices
import Foundation

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(1)
}

let keyCodes: [String: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11,
    "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21,
    "6": 22, "5": 23, "equal": 24, "=": 24, "9": 25, "7": 26, "minus": 27, "-": 27, "8": 28,
    "0": 29, "rightbracket": 30, "]": 30, "o": 31, "u": 32, "leftbracket": 33, "[": 33, "i": 34,
    "p": 35, "enter": 36, "return": 36, "l": 37, "j": 38, "quote": 39, "'": 39, "k": 40,
    "semicolon": 41, ";": 41, "backslash": 42, "\\": 42, "comma": 43, ",": 43, "slash": 44,
    "/": 44, "n": 45, "m": 46, "period": 47, ".": 47, "tab": 48, "space": 49,
    "graveaccent": 50, "backquote": 50, "`": 50, "backspace": 51, "escape": 53, "esc": 53,
    "f5": 96, "f6": 97, "f7": 98, "f3": 99, "f8": 100, "f9": 101, "f11": 103, "f10": 109,
    "f12": 111, "home": 115, "pageup": 116, "delete": 117, "f4": 118, "end": 119, "f2": 120,
    "pagedown": 121, "f1": 122, "left": 123, "right": 124, "down": 125, "up": 126,
]

func chord(_ spec: String) -> (CGKeyCode, CGEventFlags) {
    // `cmd-shift-f`, also `ctrl+shift+f`; a bare `-` or `+` is the key itself.
    var parts = spec.lowercased().split(whereSeparator: { $0 == "-" || $0 == "+" }).map(String.init)
    if spec.hasSuffix("--") || spec == "-" { parts.append("-") }
    if spec.hasSuffix("++") { parts.append("=") }
    guard let key = parts.last, let code = keyCodes[key] else { fail("unknown key in \(spec)") }
    var flags: CGEventFlags = []
    for m in parts.dropLast() {
        switch m {
        case "cmd", "meta", "command": flags.insert(.maskCommand)
        case "ctrl", "control": flags.insert(.maskControl)
        case "alt", "option", "opt": flags.insert(.maskAlternate)
        case "shift": flags.insert(.maskShift)
        default: fail("unknown modifier \(m) in \(spec)")
        }
    }
    // Arrows and the navigation keys carry these on a real keyboard.
    if [115, 116, 117, 119, 121, 123, 124, 125, 126].contains(Int(code)) {
        flags.insert(.maskSecondaryFn)
        if code >= 123 { flags.insert(.maskNumericPad) }
    }
    return (code, flags)
}

let source = CGEventSource(stateID: .hidSystemState)

func post(_ e: CGEvent?) {
    e?.post(tap: .cghidEventTap)
    usleep(8000)
}

func sendKey(_ spec: String) {
    let (code, flags) = chord(spec)
    let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true)
    down?.flags = flags
    post(down)
    let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false)
    up?.flags = flags
    post(up)
}

func typeText(_ text: String) {
    for ch in text {
        var units = Array(String(ch).utf16)
        for down in [true, false] {
            let e = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down)
            e?.flags = []
            e?.keyboardSetUnicodeString(stringLength: units.count, unicodeString: &units)
            post(e)
        }
        usleep(12000)
    }
}

/// Modifiers held for the pointer events (`click [kind] [alt+shift] x y`).
var mouseFlags: CGEventFlags = []

func mouse(_ type: CGEventType, _ p: CGPoint, _ button: CGMouseButton = .left, clicks: Int64 = 1) {
    let e = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: p, mouseButton: button)
    e?.setIntegerValueField(.mouseEventClickState, value: clicks)
    e?.flags = mouseFlags
    post(e)
}

func point(_ args: ArraySlice<String>) -> CGPoint {
    let v = args.compactMap { Double($0) }
    guard v.count >= 2 else { fail("need x y") }
    return CGPoint(x: v[0], y: v[1])
}

func mainWindow(_ pid: pid_t) -> AXUIElement {
    let app = AXUIElementCreateApplication(pid)
    var value: CFTypeRef?
    for attr in [kAXMainWindowAttribute, kAXFocusedWindowAttribute] {
        if AXUIElementCopyAttributeValue(app, attr as CFString, &value) == .success, let w = value {
            return (w as! AXUIElement)
        }
    }
    if AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value) == .success,
        let ws = value as? [AXUIElement], let w = ws.first
    {
        return w
    }
    fail("no window for pid \(pid) (Accessibility permission?)")
}

var args = Array(CommandLine.arguments.dropFirst())
guard let cmd = args.first else { fail("usage: macinput <command> ...") }
args.removeFirst()

switch cmd {
case "trusted":
    print(AXIsProcessTrusted() ? "1" : "0")
case "front":
    print(NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0)
case "windows":
    let pids = Set(args.compactMap { Int($0) })
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
        as? [[String: Any]] ?? []
    var out: [[String: Any]] = []
    for w in list {
        guard let pid = w[kCGWindowOwnerPID as String] as? Int, pids.contains(pid) else { continue }
        let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
        out.append([
            "id": w[kCGWindowNumber as String] as? Int ?? 0, "pid": pid,
            "layer": w[kCGWindowLayer as String] as? Int ?? 0,
            "title": w[kCGWindowName as String] as? String ?? "",
            "x": b["X"] ?? 0, "y": b["Y"] ?? 0, "w": b["Width"] ?? 0, "h": b["Height"] ?? 0,
        ])
    }
    let data = try! JSONSerialization.data(withJSONObject: out)
    print(String(data: data, encoding: .utf8)!)
case "place":
    guard args.count == 5, let pid = Int32(args[0]) else { fail("place <pid> <x> <y> <w> <h>") }
    let v = args.dropFirst().compactMap { Double($0) }
    let w = mainWindow(pid)
    var pos = CGPoint(x: v[0], y: v[1])
    var size = CGSize(width: v[2], height: v[3])
    AXUIElementSetAttributeValue(w, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &pos)!)
    AXUIElementSetAttributeValue(w, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &size)!)
    AXUIElementSetAttributeValue(w, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &pos)!)
case "focus":
    guard let pid = Int32(args.first ?? "") else { fail("focus <pid>") }
    NSRunningApplication(processIdentifier: pid)?.activate(options: [.activateIgnoringOtherApps])
    AXUIElementPerformAction(mainWindow(pid), kAXRaiseAction as CFString)
    usleep(200_000)
case "close":
    guard let pid = Int32(args.first ?? "") else { fail("close <pid>") }
    var button: CFTypeRef?
    if AXUIElementCopyAttributeValue(mainWindow(pid), kAXCloseButtonAttribute as CFString, &button) == .success,
        let b = button
    {
        AXUIElementPerformAction(b as! AXUIElement, kAXPressAction as CFString)
    }
case "key":
    for spec in args { sendKey(spec) }
case "type":
    typeText(args.joined(separator: " "))
case "move":
    mouse(.mouseMoved, point(args[...]))
case "down", "up":
    var rest = args[...]
    var button = CGMouseButton.left
    if rest.first == "right" { button = .right; rest = rest.dropFirst() } else if rest.first == "left" { rest = rest.dropFirst() }
    let p = point(rest)
    let type: CGEventType = button == .right
        ? (cmd == "down" ? .rightMouseDown : .rightMouseUp)
        : (cmd == "down" ? .leftMouseDown : .leftMouseUp)
    mouse(type, p, button)
case "drag":
    let v = args.compactMap { Double($0) }
    guard v.count == 4 else { fail("drag <x1> <y1> <x2> <y2>") }
    mouse(.leftMouseDragged, CGPoint(x: v[2], y: v[3]))
case "click":
    var rest = args[...]
    let kind = ["left", "right", "double"].contains(rest.first ?? "") ? rest.removeFirst() : "left"
    if let mods = rest.first, Double(mods) == nil {
        rest.removeFirst()
        for m in mods.split(separator: "+") {
            switch m {
            case "cmd": mouseFlags.insert(.maskCommand)
            case "ctrl": mouseFlags.insert(.maskControl)
            case "alt": mouseFlags.insert(.maskAlternate)
            case "shift": mouseFlags.insert(.maskShift)
            default: fail("unknown modifier \(m)")
            }
        }
    }
    let p = point(rest)
    mouse(.mouseMoved, p)
    if kind == "right" {
        mouse(.rightMouseDown, p, .right)
        mouse(.rightMouseUp, p, .right)
    } else {
        mouse(.leftMouseDown, p)
        mouse(.leftMouseUp, p)
        if kind == "double" {
            mouse(.leftMouseDown, p, clicks: 2)
            mouse(.leftMouseUp, p, clicks: 2)
        }
    }
case "wheel":
    let v = args.compactMap { Double($0) }
    guard v.count == 3 else { fail("wheel <x> <y> <lines>") }
    mouse(.mouseMoved, CGPoint(x: v[0], y: v[1]))
    let lines = Int32(v[2])
    for _ in 0..<abs(lines) {
        let e = CGEvent(scrollWheelEvent2Source: source, units: .line, wheelCount: 1,
                        wheel1: lines > 0 ? 1 : -1, wheel2: 0, wheel3: 0)
        post(e)
        usleep(40000)
    }
default:
    fail("unknown command \(cmd)")
}
