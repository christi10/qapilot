//! One device, whichever platform: Android (emulator or phone through adb + Maestro's on-device gRPC driver) or iOS
//! (iPhone or simulator through the qapilot agent). The session, the MCP tools and the design check only talk to this.
use crate::driver::Driver;
use crate::ios::{self, Ios};
use crate::screen::{self, Screen};

pub enum Device {
    Android(Driver),
    Ios(Ios),
}

impl Device {
    /// An iOS UDID selects the iPhone / simulator backend; anything else is an adb serial.
    pub fn new(serial: &str) -> Device {
        if ios::is_udid(serial) {
            Device::Ios(Ios::new(serial))
        } else {
            Device::Android(Driver::new(serial))
        }
    }

    pub fn is_ios(&self) -> bool {
        matches!(self, Device::Ios(_))
    }

    pub fn start(&self) -> Result<(), String> {
        match self {
            Device::Android(d) => d.start(),
            Device::Ios(i) => i.start(),
        }
    }

    pub fn release(&self) -> Result<(), String> {
        match self {
            Device::Android(d) => d.release(),
            Device::Ios(i) => i.release(),
        }
    }

    /// The current screen; `last_app` (the previous screen's app) helps iOS find the app in front.
    pub fn screen(&self, last_app: Option<&str>) -> Result<Screen, String> {
        match self {
            Device::Android(d) => Ok(screen::parse(&d.hierarchy()?)),
            Device::Ios(i) => {
                if !i.alive() {
                    i.start()?;
                }
                i.screen(last_app)
            }
        }
    }

    /// Tap at device pixels.
    pub fn tap(&self, x: i32, y: i32) -> Result<(), String> {
        match self {
            Device::Android(d) => d.adb.tap(x, y), // adb input: ~20 ms (the driver's gRPC tap is ~160 ms)
            Device::Ios(i) => i.tap(x, y),
        }
    }

    pub fn swipe(&self, x1: i32, y1: i32, x2: i32, y2: i32, ms: u32) -> Result<(), String> {
        match self {
            Device::Android(d) => d.adb.swipe(x1, y1, x2, y2, ms),
            Device::Ios(i) => i.swipe(x1, y1, x2, y2, ms),
        }
    }

    pub fn input_text(&self, text: &str, app: &str) -> Result<(), String> {
        match self {
            Device::Android(d) => d.input_text(text),
            Device::Ios(i) => i.input_text(text, app),
        }
    }

    pub fn erase(&self, n: u32, app: &str) -> Result<(), String> {
        match self {
            Device::Android(d) => d.erase(n),
            Device::Ios(i) => i.erase(n, app),
        }
    }

    /// `back` · `enter` · `home` (iOS: back is the edge swipe, enter the return key).
    pub fn key(&self, key: &str, app: &str) -> Result<(), String> {
        match self {
            Device::Android(d) => d.adb.keyevent(match key {
                "back" => 4,
                "enter" => 66,
                "home" => 3,
                other => return Err(format!("key {other}? (back | enter | home)")),
            }),
            Device::Ios(i) => i.key(if key == "enter" { "return" } else { key }, app),
        }
    }

    pub fn keyboard_shown(&self, last: Option<&Screen>) -> bool {
        match self {
            Device::Android(d) => d.adb.shell("dumpsys input_method | grep -m1 mInputShown").map(|s| s.contains("mInputShown=true")).unwrap_or(false),
            Device::Ios(i) => i.keyboard_shown(last),
        }
    }

    pub fn screenshot_png(&self) -> Result<Vec<u8>, String> {
        match self {
            Device::Android(d) => d.adb.exec("screencap -p"),
            Device::Ios(i) => i.screenshot_png(),
        }
    }

    /// `app`: an Android package or an iOS bundle id (already checked to be a plain identifier).
    pub fn launch(&self, app: &str) -> Result<(), String> {
        match self {
            Device::Android(d) => d.adb.shell(&format!("monkey -p {app} -c android.intent.category.LAUNCHER 1")).map(|_| ()),
            Device::Ios(i) => i.launch(app),
        }
    }

    pub fn stop(&self, app: &str) -> Result<(), String> {
        match self {
            Device::Android(d) => d.adb.shell(&format!("am force-stop {app}")).map(|_| ()),
            Device::Ios(i) => i.stop(app),
        }
    }

    /// Screen size in device pixels.
    pub fn size(&self) -> Result<(i32, i32), String> {
        match self {
            Device::Android(d) => d.device_info().map(|(w, h)| (w as i32, h as i32)),
            Device::Ios(i) => i.size(),
        }
    }

    /// "Android · Pixel 9 Pro (emulator)" / "iPhone · iPhone14,2", for pictures and reports.
    pub fn label(&self) -> String {
        match self {
            Device::Android(d) => {
                // An emulator is named by its device profile (Pixel_9_Pro → "Pixel 9 Pro"), a phone by its model.
                let avd = d.adb.shell("getprop ro.boot.qemu.avd_name").unwrap_or_default().trim().replace('_', " ");
                let emu = !avd.is_empty() || d.adb.shell("getprop ro.kernel.qemu").unwrap_or_default().trim() == "1";
                let model = if avd.is_empty() { d.adb.shell("getprop ro.product.model").unwrap_or_default().trim().to_string() } else { avd };
                format!("Android · {}{}", if model.is_empty() { "device".into() } else { model }, if emu { " (emulator)" } else { "" })
            }
            Device::Ios(i) => format!("iOS · {}", i.model()),
        }
    }

    /// Device pixels per dp / pt: Android's density / 160 (an override wins), the iPhone's screen scale.
    pub fn scale(&self) -> f32 {
        match self {
            Device::Android(d) => {
                let out = d.adb.shell("wm density").unwrap_or_default();
                let mut v = 480.0;
                for l in out.lines() {
                    if let Some(n) = l.split(':').nth(1).and_then(|x| x.trim().parse::<f32>().ok()) {
                        v = n; // "Physical density: 480" then, if set, "Override density: …"
                    }
                }
                v / 160.0
            }
            Device::Ios(i) => i.scale(),
        }
    }
}
