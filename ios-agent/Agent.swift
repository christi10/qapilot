// qapilot iOS agent — a UI test that never ends and serves the device over HTTP for qapilot on the Mac
// (port 22087; a USB iPhone is reached through usbmuxd, a simulator on localhost). Written for qapilot because
// Maestro's driver (2.4) reads the lock-screen process instead of the foreground app on iOS 27 and crashes on taps.
//
//   GET  /status                         → {"status":"ok"}
//   GET  /info                           → {"width","height" (points),"scale"}
//   POST /foreground {"apps":[ids]}      → {"app": the first of them in the foreground, else null}
//   POST /tree {"app": id} | {"apps":[…]} → the app's accessibility tree: {t,f:[x,y,w,h],id,label,value,title,ph,off,sel,c:[…]}
//   POST /tap {"x","y"[,"duration"]}     → tap (or long press) at screen points
//   POST /swipe {"x1","y1","x2","y2"[,"duration"]}
//   POST /type {"text","app"}  · POST /erase {"n","app"}  · POST /key {"key":"home"|"return","app"}
//   GET  /screenshot[?jpeg]              → PNG (or JPEG) of the screen
//   POST /launch {"app"} · POST /terminate {"app"}
import Network
import UIKit
import XCTest

final class QapilotAgent: XCTestCase {
    /// A failed interaction (e.g. typing with no focused field) must not end the test — that would stop the agent.
    /// It is kept for the request that caused it and answered as an error.
    override func record(_ issue: XCTIssue) {
        Handler.issue = issue.compactDescription
    }

    func testServe() throws {
        continueAfterFailure = true
        let env = ProcessInfo.processInfo.environment
        let port = UInt16(env["QAPILOT_PORT"] ?? "") ?? 22087
        let server = try Server(port: port)
        server.start()
        NSLog("qapilot agent listening on port %d", Int(port))
        while true {
            RunLoop.main.run(until: Date().addingTimeInterval(1))
        }
    }
}

final class Server {
    let listener: NWListener

    init(port: UInt16) throws {
        let params = NWParameters.tcp
        params.allowLocalEndpointReuse = true
        listener = try NWListener(using: params, on: NWEndpoint.Port(rawValue: port)!)
    }

    func start() {
        listener.newConnectionHandler = { conn in
            conn.start(queue: .global(qos: .userInitiated))
            Exchange(conn).read(Data())
        }
        listener.start(queue: .global(qos: .userInitiated))
    }
}

/// One HTTP request → one response, then the connection is closed.
final class Exchange {
    let conn: NWConnection
    init(_ c: NWConnection) { conn = c }

    func read(_ buf: Data) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 1 << 20) { data, _, done, err in
            var buf = buf
            if let d = data { buf.append(d) }
            if let r = buf.range(of: Data("\r\n\r\n".utf8)) {
                let head = String(decoding: buf[..<r.lowerBound], as: UTF8.self)
                let len = head.split(separator: "\r\n").first { $0.lowercased().hasPrefix("content-length:") }
                    .flatMap { Int($0.split(separator: ":")[1].trimmingCharacters(in: .whitespaces)) } ?? 0
                let body = buf[r.upperBound...]
                if body.count >= len {
                    let line = head.split(separator: "\r\n").first.map(String.init) ?? ""
                    let parts = line.split(separator: " ")
                    let method = parts.count > 0 ? String(parts[0]) : "GET"
                    let path = parts.count > 1 ? String(parts[1]) : "/"
                    var res: (Int, String, Data) = (500, "text/plain", Data())
                    DispatchQueue.main.sync { res = Handler.handle(method, path, Data(body.prefix(len))) }
                    self.respond(res)
                    return
                }
            }
            if done || err != nil { self.conn.cancel(); return }
            self.read(buf)
        }
    }

    func respond(_ r: (Int, String, Data)) {
        let reason = r.0 == 200 ? "OK" : "Error"
        var out = Data("HTTP/1.1 \(r.0) \(reason)\r\nContent-Type: \(r.1)\r\nContent-Length: \(r.2.count)\r\nConnection: close\r\n\r\n".utf8)
        out.append(r.2)
        conn.send(content: out, completion: .contentProcessed { _ in self.conn.cancel() })
    }
}

enum Handler {
    static let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    static var issue: String?

    static func json(_ o: Any, _ code: Int = 200) -> (Int, String, Data) {
        let d = (try? JSONSerialization.data(withJSONObject: o, options: [])) ?? Data("{}".utf8)
        return (code, "application/json", d)
    }

    static func fail(_ msg: String) -> (Int, String, Data) { json(["error": msg], 400) }

    static func handle(_ method: String, _ path: String, _ body: Data) -> (Int, String, Data) {
        issue = nil
        let res = route(method, path, body)
        if let i = issue { return fail(i) }
        return res
    }

    static func route(_ method: String, _ path: String, _ body: Data) -> (Int, String, Data) {
        let req = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any] ?? [:]
        let route = String(path.split(separator: "?").first ?? "/")
        let num = { (k: String) -> CGFloat? in (req[k] as? NSNumber).map { CGFloat(truncating: $0) } }
        let app = { () -> XCUIApplication in
            if let id = req["app"] as? String, !id.isEmpty { return XCUIApplication(bundleIdentifier: id) }
            return foreground(req["apps"] as? [String] ?? []) ?? springboard
        }
        switch route {
        case "/status":
            return json(["status": "ok"])
        case "/info":
            let b = UIScreen.main.bounds
            var sys = utsname()
            uname(&sys)
            let model = withUnsafePointer(to: &sys.machine) { $0.withMemoryRebound(to: CChar.self, capacity: 1) { String(cString: $0) } }
            return json(["width": b.width, "height": b.height, "scale": UIScreen.main.scale, "model": model, "os": UIDevice.current.systemVersion])
        case "/foreground":
            let ids = req["apps"] as? [String] ?? []
            let id = ids.first { XCUIApplication(bundleIdentifier: $0).state == .runningForeground }
            return json(["app": id.map { $0 as Any } ?? NSNull()])
        case "/tree":
            do {
                let a = app()
                let snap = try a.snapshot()
                var out: [String: Any] = ["app": (req["app"] as? String).map { $0 as Any } ?? NSNull(), "tree": node(snap)]
                // A system alert (permissions, "Install", …) belongs to SpringBoard, not to the app in front.
                let alert = springboard.alerts.firstMatch
                if alert.exists, let s = try? alert.snapshot() { out["alert"] = node(s) }
                return json(out)
            } catch {
                return fail("snapshot failed: \(error.localizedDescription)")
            }
        case "/tap":
            guard let x = num("x"), let y = num("y") else { return fail("x, y missing") }
            let c = point(x, y)
            let d = Double(num("duration") ?? 0)
            if d >= 0.3 { c.press(forDuration: d) } else { c.tap() }
            return json(["ok": true])
        case "/swipe":
            guard let x1 = num("x1"), let y1 = num("y1"), let x2 = num("x2"), let y2 = num("y2") else { return fail("x1, y1, x2, y2 missing") }
            let d = max(Double(num("duration") ?? 0.3), 0.05)
            let dist = hypot(x2 - x1, y2 - y1)
            point(x1, y1).press(forDuration: 0.05, thenDragTo: point(x2, y2), withVelocity: XCUIGestureVelocity(rawValue: max(dist / CGFloat(d), 100)), thenHoldForDuration: 0.05)
            return json(["ok": true])
        case "/type":
            guard let t = req["text"] as? String else { return fail("text missing") }
            app().typeText(t)
            return json(["ok": true])
        case "/erase":
            let n = Int(num("n") ?? 50)
            app().typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: max(n, 0)))
            return json(["ok": true])
        case "/key":
            switch req["key"] as? String ?? "" {
            case "home": XCUIDevice.shared.press(.home)
            case "return", "enter": app().typeText("\n")
            default: return fail("key: home | return")
            }
            return json(["ok": true])
        case "/screenshot":
            let shot = XCUIScreen.main.screenshot()
            if path.contains("jpeg"), let j = shot.image.jpegData(compressionQuality: 0.6) { return (200, "image/jpeg", j) }
            return (200, "image/png", shot.pngRepresentation)
        case "/launch":
            guard let id = req["app"] as? String else { return fail("app missing") }
            XCUIApplication(bundleIdentifier: id).activate()
            return json(["ok": true])
        case "/terminate":
            guard let id = req["app"] as? String else { return fail("app missing") }
            XCUIApplication(bundleIdentifier: id).terminate()
            return json(["ok": true])
        default:
            return fail("unknown route \(route)")
        }
    }

    /// Screen points as a coordinate (SpringBoard's frame is the whole screen; the touch goes to whatever is on top).
    static func point(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
        springboard.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0)).withOffset(CGVector(dx: x, dy: y))
    }

    static func foreground(_ ids: [String]) -> XCUIApplication? {
        ids.lazy.map { XCUIApplication(bundleIdentifier: $0) }.first { $0.state == .runningForeground }
    }

    static func node(_ s: XCUIElementSnapshot) -> [String: Any] {
        let f = s.frame
        var d: [String: Any] = ["t": s.elementType.rawValue, "f": [f.minX, f.minY, f.width, f.height]]
        if !s.identifier.isEmpty { d["id"] = s.identifier }
        if !s.label.isEmpty { d["label"] = s.label }
        if !s.title.isEmpty { d["title"] = s.title }
        if let v = s.value, !(v is NSNull) {
            let str = (v as? String) ?? "\(v)"
            if !str.isEmpty { d["value"] = str }
        }
        if let p = s.placeholderValue, !p.isEmpty { d["ph"] = p }
        if !s.isEnabled { d["off"] = true }
        if s.isSelected { d["sel"] = true }
        let kids = s.children.map(node)
        if !kids.isEmpty { d["c"] = kids }
        return d
    }
}
