//! The adb server's own protocol on 127.0.0.1:5037 — no `adb` process per call.
//! A request is `<4 hex digits length><payload>`; the server answers `OKAY` or `FAIL<4 hex len><message>`.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct Adb {
    pub serial: String,
}

/// The `adb` binary, for the rare things its server socket does not do (emulator console, installing an APK).
pub fn bin() -> String {
    let sdk = std::env::var("ANDROID_HOME").unwrap_or_else(|_| format!("{}/Library/Android/sdk", std::env::var("HOME").unwrap_or_default()));
    format!("{sdk}/platform-tools/adb")
}

fn send(s: &mut TcpStream, msg: &str) -> Result<(), String> {
    s.write_all(format!("{:04x}{}", msg.len(), msg).as_bytes()).map_err(|e| format!("adb write: {e}"))?;
    status(s)
}

fn status(s: &mut TcpStream) -> Result<(), String> {
    let mut st = [0u8; 4];
    s.read_exact(&mut st).map_err(|e| format!("adb read: {e}"))?;
    match &st {
        b"OKAY" => Ok(()),
        b"FAIL" => {
            let mut len = [0u8; 4];
            s.read_exact(&mut len).map_err(|e| e.to_string())?;
            let n = usize::from_str_radix(std::str::from_utf8(&len).unwrap_or("0"), 16).unwrap_or(0);
            let mut m = vec![0u8; n];
            s.read_exact(&mut m).map_err(|e| e.to_string())?;
            Err(format!("adb: {}", String::from_utf8_lossy(&m)))
        }
        other => Err(format!("adb: unexpected reply {:?}", String::from_utf8_lossy(other))),
    }
}

fn connect() -> Result<TcpStream, String> {
    let s = TcpStream::connect_timeout(&"127.0.0.1:5037".parse().unwrap(), Duration::from_secs(3))
        .map_err(|e| format!("adb server not reachable on 5037 ({e}) — start it with `adb start-server`"))?;
    s.set_read_timeout(Some(Duration::from_secs(60))).ok();
    Ok(s)
}

impl Adb {
    pub fn new(serial: &str) -> Self {
        Adb { serial: serial.to_string() }
    }

    fn device(&self) -> Result<TcpStream, String> {
        let mut s = connect()?;
        send(&mut s, &format!("host:transport:{}", self.serial))?;
        Ok(s)
    }

    /// Raw bytes of a command run on the device (`exec:` — no tty, binary-safe).
    pub fn exec(&self, cmd: &str) -> Result<Vec<u8>, String> {
        let mut s = self.device()?;
        send(&mut s, &format!("exec:{cmd}"))?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).map_err(|e| format!("adb exec: {e}"))?;
        Ok(out)
    }

    /// Text output of a shell command on the device.
    pub fn shell(&self, cmd: &str) -> Result<String, String> {
        let mut s = self.device()?;
        send(&mut s, &format!("shell:{cmd}"))?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).map_err(|e| format!("adb shell: {e}"))?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Starts a background command (`… &`) through the shell v2 service — the legacy `shell:`/`exec:` services take a
    /// backgrounded `am instrument` down with them when the connection closes, v2 (what `adb shell` uses) does not.
    pub fn spawn(&self, cmd: &str) -> Result<(), String> {
        let mut s = self.device()?;
        send(&mut s, &format!("shell,v2,raw:{cmd}"))?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).map_err(|e| format!("adb shell: {e}"))?;
        Ok(())
    }

    /// `adb forward tcp:<local> tcp:<remote>` (kept by the adb server until removed).
    pub fn forward(&self, local: u16, remote: u16) -> Result<(), String> {
        let mut s = connect()?;
        send(&mut s, &format!("host-serial:{}:forward:tcp:{};tcp:{}", self.serial, local, remote))?;
        status(&mut s).or(Ok(()))
    }

    pub fn keyevent(&self, code: u32) -> Result<(), String> {
        self.shell(&format!("input keyevent {code}")).map(|_| ())
    }

    pub fn tap(&self, x: i32, y: i32) -> Result<(), String> {
        self.shell(&format!("input tap {x} {y}")).map(|_| ())
    }

    pub fn swipe(&self, x1: i32, y1: i32, x2: i32, y2: i32, ms: u32) -> Result<(), String> {
        self.shell(&format!("input swipe {x1} {y1} {x2} {y2} {ms}")).map(|_| ())
    }
}
