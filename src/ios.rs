//! iPhone and iOS Simulator backend: talks HTTP to the qapilot agent (ios-agent/Agent.swift, an XCTest that keeps
//! running on the device) — over usbmuxd for a USB iPhone, on localhost for a simulator. Element bounds are kept in
//! device pixels (points × scale) like Android, so selectors, taps, fingerprints and the design check work unchanged.
use crate::screen::{El, Screen};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const PORT: u16 = 22087;
const SPRINGBOARD: &str = "com.apple.springboard";

/// `00008110-0012345678ABCDEF` (iPhone) or `60E1D21F-1649-4D55-ABFB-84E7960019B6` (simulator).
pub fn is_udid(s: &str) -> bool {
    let hex = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_hexdigit());
    let parts: Vec<&str> = s.split('-').collect();
    (parts.len() == 2 && parts[0].len() == 8 && parts[1].len() == 16 && parts.iter().all(|p| hex(p)))
        || (parts.len() == 5 && [8, 4, 4, 4, 12].iter().zip(&parts).all(|(n, p)| p.len() == *n && hex(p)))
        || (s.len() == 40 && hex(s))
}

pub struct Ios {
    pub udid: String,
    sim: bool,
    scale: std::cell::Cell<f32>,
    size: std::cell::Cell<(i32, i32)>,
}

// usbmuxd speaks length-prefixed XML plists over a Unix socket.
fn mux_packet(body: &str, tag: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(16 + body.len());
    for n in [16 + body.len() as u32, 1, 8, tag] {
        v.extend_from_slice(&n.to_le_bytes());
    }
    v.extend_from_slice(body.as_bytes());
    v
}

fn mux_reply(s: &mut UnixStream) -> Result<String, String> {
    let mut head = [0u8; 16];
    s.read_exact(&mut head).map_err(|e| format!("usbmuxd: {e}"))?;
    let n = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let mut body = vec![0u8; n.saturating_sub(16)];
    s.read_exact(&mut body).map_err(|e| format!("usbmuxd: {e}"))?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn plist(kv: &[(&str, &str)]) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>");
    for (k, v) in kv {
        if let Some(n) = v.strip_prefix("int:") {
            s.push_str(&format!("<key>{k}</key><integer>{n}</integer>"));
        } else {
            s.push_str(&format!("<key>{k}</key><string>{v}</string>"));
        }
    }
    s.push_str("</dict></plist>");
    s
}

/// The usbmuxd DeviceID of a USB-connected iPhone.
fn mux_device(udid: &str) -> Result<u64, String> {
    let mut s = UnixStream::connect("/var/run/usbmuxd").map_err(|e| format!("usbmuxd: {e}"))?;
    s.write_all(&mux_packet(&plist(&[("MessageType", "ListDevices"), ("ClientVersionString", "qapilot"), ("ProgName", "qapilot")]), 1)).map_err(|e| e.to_string())?;
    let reply = mux_reply(&mut s)?;
    let want = udid.replace('-', "").to_lowercase();
    // Each device: <key>DeviceID</key><integer>N</integer> … <key>SerialNumber</key><string>…</string>
    for chunk in reply.split("<key>DeviceID</key>").skip(1) {
        let id = chunk.split("<integer>").nth(1).and_then(|x| x.split('<').next()).and_then(|x| x.parse::<u64>().ok());
        let serial = chunk.split("<key>SerialNumber</key>").nth(1).and_then(|x| x.split("<string>").nth(1)).and_then(|x| x.split('<').next()).unwrap_or("");
        if let Some(id) = id {
            if serial.replace('-', "").to_lowercase() == want {
                return Ok(id);
            }
        }
    }
    Err(format!("iPhone {udid} is not connected over USB (usbmuxd does not list it)"))
}

enum Conn {
    Mux(UnixStream),
    Tcp(std::net::TcpStream),
}

impl Read for Conn {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Mux(s) => s.read(b),
            Conn::Tcp(s) => s.read(b),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Mux(s) => s.write(b),
            Conn::Tcp(s) => s.write(b),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Mux(s) => s.flush(),
            Conn::Tcp(s) => s.flush(),
        }
    }
}

/// UIKit element types (XCUIElement.ElementType raw values) that take taps.
fn tappable(t: i64) -> bool {
    matches!(t, 9 | 10 | 12 | 14 | 16 | 17 | 20 | 33 | 37 | 39 | 40 | 41 | 42 | 44 | 45 | 49 | 50 | 52 | 54 | 75 | 79 | 80)
}

fn type_name(t: i64) -> &'static str {
    match t {
        2 => "Application",
        3 => "Group",
        4 => "Window",
        7 => "Alert",
        9 => "Button",
        12 => "CheckBox",
        19 => "Keyboard",
        20 => "Key",
        21 => "NavigationBar",
        22 => "TabBar",
        26 => "Table",
        32 => "CollectionView",
        40 => "Switch",
        42 => "Link",
        43 => "Image",
        45 => "SearchField",
        46 => "ScrollView",
        48 => "StaticText",
        49 => "TextField",
        50 => "SecureTextField",
        52 => "TextView",
        58 => "WebView",
        75 => "Cell",
        80 => "Tab",
        _ => "Other",
    }
}

impl Ios {
    pub fn new(udid: &str) -> Ios {
        let sim = Command::new("xcrun").args(["simctl", "list", "devices"]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains(udid)).unwrap_or(false);
        Ios { udid: udid.to_string(), sim, scale: std::cell::Cell::new(0.0), size: std::cell::Cell::new((0, 0)) }
    }

    fn connect(&self) -> Result<Conn, String> {
        if self.sim {
            let s = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], PORT).into(), Duration::from_secs(2)).map_err(|e| format!("agent: {e}"))?;
            return Ok(Conn::Tcp(s));
        }
        let id = mux_device(&self.udid)?;
        let mut s = UnixStream::connect("/var/run/usbmuxd").map_err(|e| format!("usbmuxd: {e}"))?;
        let port = PORT.to_be() as u32; // usbmuxd wants the port in network byte order
        s.write_all(&mux_packet(&plist(&[("MessageType", "Connect"), ("ClientVersionString", "qapilot"), ("ProgName", "qapilot"), ("DeviceID", &format!("int:{id}")), ("PortNumber", &format!("int:{port}"))]), 2))
            .map_err(|e| e.to_string())?;
        let reply = mux_reply(&mut s)?;
        let code = reply.split("<key>Number</key>").nth(1).and_then(|x| x.split("<integer>").nth(1)).and_then(|x| x.split('<').next()).unwrap_or("?");
        if code != "0" {
            return Err(format!("the qapilot agent is not running on the iPhone (usbmuxd connect: {code})"));
        }
        Ok(Conn::Mux(s))
    }

    /// One HTTP request to the agent; returns the body (an error for a non-200 answer).
    fn http(&self, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Vec<u8>, String> {
        let mut c = self.connect()?;
        match &mut c {
            Conn::Mux(s) => {
                let _ = s.set_read_timeout(Some(timeout));
            }
            Conn::Tcp(s) => {
                let _ = s.set_read_timeout(Some(timeout));
            }
        }
        let data = body.map(|b| b.to_string()).unwrap_or_default();
        let req = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}", data.len());
        c.write_all(req.as_bytes()).map_err(|e| format!("agent: {e}"))?;
        let mut raw = Vec::new();
        c.read_to_end(&mut raw).map_err(|e| format!("agent {path}: {e}"))?;
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or(format!("agent {path}: no answer"))?;
        let head = String::from_utf8_lossy(&raw[..split]).into_owned();
        let payload = raw[split + 4..].to_vec();
        let status = head.split_whitespace().nth(1).unwrap_or("0");
        if status != "200" {
            return Err(format!("agent {path}: HTTP {status} {}", String::from_utf8_lossy(&payload)));
        }
        Ok(payload)
    }

    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let b = self.http("POST", path, Some(&body), Duration::from_secs(60))?;
        serde_json::from_slice(&b).map_err(|e| format!("agent {path}: {e}"))
    }

    pub fn alive(&self) -> bool {
        self.http("GET", "/status", None, Duration::from_secs(3)).is_ok()
    }

    fn agent_dir(&self) -> PathBuf {
        crate::home().join("ios-agent").join(&self.udid)
    }

    /// Builds the agent for this device once (ios-agent/build.sh), starts it with xcodebuild in the background and
    /// waits until it answers (the first start on a phone takes up to ~2 min while Xcode prepares it).
    pub fn start(&self) -> Result<(), String> {
        if self.alive() {
            return Ok(());
        }
        let dir = self.agent_dir();
        let xctestrun = dir.join("agent.xctestrun");
        if !xctestrun.exists() {
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ios-agent").join("build.sh");
            let out = Command::new("bash").arg(&script).arg(&self.udid).output().map_err(|e| format!("build.sh: {e}"))?;
            if !out.status.success() {
                return Err(format!("building the iOS agent failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
            }
        }
        let log = std::fs::File::create(dir.join("xcodebuild.log")).map_err(|e| e.to_string())?;
        Command::new("xcodebuild")
            .args(["test-without-building", "-xctestrun"])
            .arg(&xctestrun)
            .arg("-destination")
            .arg(format!("id={}", self.udid))
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("xcodebuild: {e}"))?;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(180) {
            std::thread::sleep(Duration::from_millis(500));
            if self.alive() {
                return Ok(());
            }
            let text = std::fs::read_to_string(dir.join("xcodebuild.log")).unwrap_or_default();
            if text.contains("TEST EXECUTE FAILED") || text.contains("xcodebuild: error") {
                return Err(format!("the iOS agent did not start — see {}", dir.join("xcodebuild.log").display()));
            }
        }
        Err("the iOS agent did not answer within 3 min (is the phone unlocked?)".into())
    }

    pub fn release(&self) -> Result<(), String> {
        let _ = Command::new("pkill").args(["-f", &format!("{}/agent.xctestrun", self.udid)]).status();
        Ok(())
    }

    fn info(&self) -> Result<(f32, (i32, i32)), String> {
        if self.scale.get() > 0.0 {
            return Ok((self.scale.get(), self.size.get()));
        }
        let v = self.post("/info", json!({})).or_else(|_| {
            let b = self.http("GET", "/info", None, Duration::from_secs(10))?;
            serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string())
        })?;
        let s = v["scale"].as_f64().unwrap_or(3.0) as f32;
        let (w, h) = (v["width"].as_f64().unwrap_or(390.0) as f32, v["height"].as_f64().unwrap_or(844.0) as f32);
        self.scale.set(s);
        self.size.set(((w * s) as i32, (h * s) as i32));
        Ok((s, self.size.get()))
    }

    /// The phone's model from the agent ("iPhone14,2"), else "iPhone".
    pub fn model(&self) -> String {
        self.http("GET", "/info", None, Duration::from_secs(10)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["model"].as_str().map(str::to_string)).unwrap_or_else(|| "iPhone".into())
    }

    pub fn scale(&self) -> f32 {
        self.info().map(|i| i.0).unwrap_or(3.0)
    }

    /// Screen size in device pixels.
    pub fn size(&self) -> Result<(i32, i32), String> {
        self.info().map(|i| i.1)
    }

    /// Apps that may be in front: $QAPILOT_IOS_APPS (comma separated), the last screen's app, Safari, Settings.
    fn candidates(&self, last: Option<&str>) -> Vec<String> {
        let mut v: Vec<String> = std::env::var("QAPILOT_IOS_APPS").unwrap_or_default().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if let Some(l) = last.filter(|l| !l.is_empty() && *l != SPRINGBOARD) {
            v.push(l.to_string());
        }
        v.extend(["com.apple.mobilesafari".to_string(), "com.apple.Preferences".to_string()]);
        v.dedup();
        v
    }

    pub fn foreground(&self, last: Option<&str>) -> String {
        self.post("/foreground", json!({ "apps": self.candidates(last) })).ok().and_then(|v| v["app"].as_str().map(str::to_string)).unwrap_or_else(|| SPRINGBOARD.to_string())
    }

    /// The front app's screen as qapilot's model: on-screen elements that carry an id, text or a tap, in pixels.
    pub fn screen(&self, last: Option<&str>) -> Result<Screen, String> {
        let (scale, (w, h)) = self.info()?;
        let app = self.foreground(last);
        let v = self.post("/tree", json!({ "app": app }))?;
        let mut out = Screen { package: app.clone(), width: w, height: h, ..Default::default() };
        fn walk(n: &Value, scale: f32, w: i32, h: i32, app: &str, out: &mut Screen) {
            let f: Vec<f32> = n["f"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
            if f.len() == 4 && f[2] > 0.0 && f[3] > 0.0 {
                let b = [(f[0] * scale).round() as i32, (f[1] * scale).round() as i32, ((f[0] + f[2]) * scale).round() as i32, ((f[1] + f[3]) * scale).round() as i32];
                let t = n["t"].as_i64().unwrap_or(0);
                let s = |k: &str| n[k].as_str().unwrap_or("").to_string();
                let (label, value, title, id) = (s("label"), s("value"), s("title"), s("id"));
                let text = if !label.is_empty() { label.clone() } else { title.clone() };
                // A password / PIN field's value never leaves the tool (iOS reports it in clear while it is edited).
                let secret = t == 50 || (matches!(t, 49 | 52) && [&id, &label].iter().any(|k| { let k = k.to_lowercase(); k.contains("password") || k.contains("pin") || k.contains("passcode") }));
                let value = if secret && !value.is_empty() { "•".repeat(value.chars().count()) } else { value };
                let desc = if !value.is_empty() && value != text { value } else { s("ph") };
                let on_screen = b[2] > 0 && b[3] > 0 && b[0] < w && b[1] < h;
                if t == 36 && on_screen {
                    out.busy = true; // an activity indicator: the screen is still loading
                }
                let meaningful = !text.is_empty() || !desc.is_empty() || !id.is_empty() || tappable(t);
                if on_screen && meaningful && t != 2 {
                    out.elements.push(El {
                        r#ref: String::new(),
                        id,
                        text,
                        desc,
                        class: type_name(t).to_string(),
                        package: app.to_string(),
                        clickable: tappable(t),
                        checked: n["sel"].as_bool().unwrap_or(false),
                        enabled: !n["off"].as_bool().unwrap_or(false),
                        focused: false,
                        password: secret,
                        bounds: b,
                    });
                }
            }
            if let Some(kids) = n["c"].as_array() {
                for k in kids {
                    walk(k, scale, w, h, app, out);
                }
            }
        }
        walk(&v["tree"], scale, w, h, &app, &mut out);
        if v["alert"].is_object() {
            walk(&v["alert"], scale, w, h, SPRINGBOARD, &mut out);
        }
        Ok(crate::screen::finish(out))
    }

    fn pt(&self, px: i32) -> f32 {
        px as f32 / self.scale()
    }

    pub fn tap(&self, x: i32, y: i32) -> Result<(), String> {
        self.post("/tap", json!({ "x": self.pt(x), "y": self.pt(y) })).map(|_| ())
    }

    pub fn swipe(&self, x1: i32, y1: i32, x2: i32, y2: i32, ms: u32) -> Result<(), String> {
        self.post("/swipe", json!({ "x1": self.pt(x1), "y1": self.pt(y1), "x2": self.pt(x2), "y2": self.pt(y2), "duration": ms as f32 / 1000.0 })).map(|_| ())
    }

    /// One character at a time: React Native inputs on iOS drop characters typed in one burst.
    pub fn input_text(&self, text: &str, app: &str) -> Result<(), String> {
        for c in text.chars() {
            self.post("/type", json!({ "text": c.to_string(), "app": app }))?;
            std::thread::sleep(Duration::from_millis(60));
        }
        Ok(())
    }

    pub fn erase(&self, n: u32, app: &str) -> Result<(), String> {
        self.post("/erase", json!({ "n": n, "app": app })).map(|_| ())
    }

    pub fn key(&self, key: &str, app: &str) -> Result<(), String> {
        match key {
            // iOS has no back key: the system edge swipe goes back in navigation stacks.
            "back" => {
                let (w, h) = self.size()?;
                self.swipe(2, h / 2, w * 7 / 10, h / 2, 250)
            }
            "home" | "return" => self.post("/key", json!({ "key": key, "app": app })).map(|_| ()),
            other => Err(format!("key {other}? (back | home | return)")),
        }
    }

    pub fn keyboard_shown(&self, last: Option<&Screen>) -> bool {
        last.map(|s| s.elements.iter().any(|e| e.class == "Keyboard" || e.class == "Key")).unwrap_or(false)
    }

    pub fn screenshot_png(&self) -> Result<Vec<u8>, String> {
        self.http("GET", "/screenshot", None, Duration::from_secs(30))
    }

    pub fn launch(&self, app: &str) -> Result<(), String> {
        self.post("/launch", json!({ "app": app })).map(|_| ())
    }

    pub fn stop(&self, app: &str) -> Result<(), String> {
        self.post("/terminate", json!({ "app": app })).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_iphone_and_simulator_udids() {
        assert!(is_udid("00008110-0012345678ABCDEF"));
        assert!(is_udid("60E1D21F-1649-4D55-ABFB-84E7960019B6"));
        assert!(!is_udid("emulator-5554"));
        assert!(!is_udid("192.168.1.20:5555"));
    }

    #[test]
    fn usbmux_packets_are_little_endian_plists() {
        let p = mux_packet(&plist(&[("MessageType", "Connect"), ("DeviceID", "int:4")]), 2);
        assert_eq!(u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize, p.len());
        assert_eq!(u32::from_le_bytes([p[12], p[13], p[14], p[15]]), 2);
        let body = String::from_utf8_lossy(&p[16..]);
        assert!(body.contains("<key>DeviceID</key><integer>4</integer>") && body.contains("<string>Connect</string>"));
    }
}
