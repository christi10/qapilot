//! Maestro's on-device driver (dev.mobile.maestro + dev.mobile.maestro.test — installed from the local Maestro when the
//! device does not have it), spoken to directly over gRPC (HTTP/2) — without the Maestro CLI's JVM. The service is `maestro_android.MaestroDriver`; the
//! messages are tiny, so they are encoded by hand (see `pb`).
use crate::adb::Adb;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::time::{Duration, Instant};

/// Device port the driver listens on (Maestro's CLI uses 7001; another port so both can coexist in config).
pub const DEVICE_PORT: u16 = 7101;
/// Local port forwarded to it.
pub const LOCAL_PORT: u16 = 7101;

const INSTRUMENT: &str = "am instrument -w -e port 7101 -e class 'dev.mobile.maestro.MaestroDriverService#grpcServer' dev.mobile.maestro.test/androidx.test.runner.AndroidJUnitRunner";

/// The driver's two APKs inside Maestro's client jar.
const DRIVER_APKS: [&str; 2] = ["maestro-app.apk", "maestro-server.apk"];

/// `maestro-client.jar` of the Maestro on this machine: next to the `maestro` on PATH (Homebrew's is a link into the
/// Cellar), else the usual install places.
fn maestro_jar() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let on_path = std::env::var("PATH").unwrap_or_default().split(':').map(|d| std::path::Path::new(d).join("maestro")).collect::<Vec<_>>();
    let known = ["/opt/homebrew/bin/maestro".into(), "/usr/local/bin/maestro".into(), format!("{home}/.maestro/bin/maestro")].map(std::path::PathBuf::from);
    on_path.into_iter().chain(known).filter_map(|bin| std::fs::canonicalize(bin).ok()).find_map(|bin| {
        let jar = bin.parent()?.parent()?.join("lib").join("maestro-client.jar");
        jar.is_file().then_some(jar)
    })
}

/// Minimal protobuf: varint / length-delimited fields, enough for this service.
pub mod pb {
    use super::*;
    pub fn varint(b: &mut BytesMut, mut v: u64) {
        while v >= 0x80 {
            b.put_u8((v as u8) | 0x80);
            v >>= 7;
        }
        b.put_u8(v as u8);
    }
    pub fn uint(b: &mut BytesMut, field: u32, v: u64) {
        varint(b, (field as u64) << 3); // wire type 0
        varint(b, v);
    }
    pub fn string(b: &mut BytesMut, field: u32, s: &[u8]) {
        varint(b, ((field as u64) << 3) | 2);
        varint(b, s.len() as u64);
        b.put_slice(s);
    }
    fn read_varint(b: &mut Bytes) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            if !b.has_remaining() {
                return None;
            }
            let x = b.get_u8();
            v |= ((x & 0x7f) as u64) << shift;
            if x < 0x80 {
                return Some(v);
            }
        }
        None
    }
    /// Field number → value (varints as numbers, length-delimited as bytes). Unknown wire types stop the scan.
    pub fn fields(mut b: Bytes) -> Vec<(u32, Result<u64, Bytes>)> {
        let mut out = Vec::new();
        while b.has_remaining() {
            let Some(tag) = read_varint(&mut b) else { break };
            match tag & 7 {
                0 => match read_varint(&mut b) {
                    Some(v) => out.push(((tag >> 3) as u32, Ok(v))),
                    None => break,
                },
                2 => {
                    let Some(n) = read_varint(&mut b) else { break };
                    if b.remaining() < n as usize {
                        break;
                    }
                    out.push(((tag >> 3) as u32, Err(b.split_to(n as usize))));
                }
                _ => break,
            }
        }
        out
    }
}

pub struct Driver {
    pub adb: Adb,
    rt: tokio::runtime::Runtime,
}

impl Driver {
    pub fn new(serial: &str) -> Self {
        Driver { adb: Adb::new(serial), rt: tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime") }
    }

    async fn call_once(method: &str, body: Bytes, timeout: Duration) -> Result<Bytes, String> {
        let io = tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(("127.0.0.1", LOCAL_PORT)))
            .await
            .map_err(|_| "driver connect timeout".to_string())?
            .map_err(|e| format!("driver connect: {e}"))?;
        let (client, conn) = h2::client::handshake(io).await.map_err(|e| format!("http2: {e}"))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let mut client = client.ready().await.map_err(|e| format!("http2: {e}"))?;
        let req = http::Request::builder()
            .method("POST")
            .uri(format!("http://127.0.0.1:{LOCAL_PORT}/maestro_android.MaestroDriver/{method}"))
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(())
            .unwrap();
        let (resp, mut send) = client.send_request(req, false).map_err(|e| format!("grpc send: {e}"))?;
        let mut framed = BytesMut::with_capacity(5 + body.len());
        framed.put_u8(0);
        framed.put_u32(body.len() as u32);
        framed.put_slice(&body);
        send.send_data(framed.freeze(), true).map_err(|e| format!("grpc send: {e}"))?;
        let work = async {
            let resp = resp.await.map_err(|e| format!("grpc: {e}"))?;
            let (parts, mut recv) = resp.into_parts();
            if let Some(s) = parts.headers.get("grpc-status") {
                if s != "0" {
                    let m = parts.headers.get("grpc-message").and_then(|v| v.to_str().ok()).unwrap_or("");
                    return Err(format!("grpc status {}: {m}", s.to_str().unwrap_or("?")));
                }
            }
            let mut data = BytesMut::new();
            while let Some(chunk) = recv.data().await {
                let chunk = chunk.map_err(|e| format!("grpc body: {e}"))?;
                let _ = recv.flow_control().release_capacity(chunk.len());
                data.extend_from_slice(&chunk);
            }
            if let Ok(Some(t)) = recv.trailers().await {
                if let Some(s) = t.get("grpc-status") {
                    if s != "0" {
                        let m = t.get("grpc-message").and_then(|v| v.to_str().ok()).unwrap_or("");
                        return Err(format!("grpc status {}: {m}", s.to_str().unwrap_or("?")));
                    }
                }
            }
            if data.len() < 5 {
                return Ok(Bytes::new());
            }
            let mut d = data.freeze();
            d.advance(1);
            let n = d.get_u32() as usize;
            Ok(d.split_to(n.min(d.len())))
        };
        tokio::time::timeout(timeout, work).await.map_err(|_| format!("{method}: timeout"))?
    }

    fn call(&self, method: &str, body: Bytes, timeout: Duration) -> Result<Bytes, String> {
        if let Ok(b) = self.rt.block_on(Self::call_once(method, body.clone(), timeout)) {
            return Ok(b);
        }
        // Not running (first call, emulator restarted, or a Maestro replay took the device): start it, retry once.
        self.start()?;
        if let Ok(b) = self.rt.block_on(Self::call_once(method, body.clone(), timeout)) {
            return Ok(b);
        }
        // It answers `deviceInfo` but not this call (a driver left half-dead by a broken pipe): `start` sees it alive
        // and keeps it, so stop it for real and start a new one before the last try.
        self.release()?;
        std::thread::sleep(Duration::from_millis(300));
        self.start()?;
        self.rt.block_on(Self::call_once(method, body, timeout))
    }

    pub fn alive(&self) -> bool {
        self.rt.block_on(Self::call_once("deviceInfo", Bytes::new(), Duration::from_secs(2))).is_ok()
    }

    /// Starts the driver's instrumentation (backgrounded on the device) and waits until it answers.
    pub fn start(&self) -> Result<(), String> {
        self.adb.forward(LOCAL_PORT, DEVICE_PORT)?;
        if self.alive() {
            return Ok(());
        }
        if !self.installed()? {
            self.install()?;
        }
        // nohup: without it the instrumentation dies with the adb shell session that started it.
        self.adb.spawn(&format!("nohup {INSTRUMENT} >/dev/null 2>&1 &"))?;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(25) {
            std::thread::sleep(Duration::from_millis(250));
            if self.alive() {
                return Ok(());
            }
        }
        Err("the on-device driver did not start within 25 s".into())
    }

    fn installed(&self) -> Result<bool, String> {
        let pkgs = self.adb.shell("pm list packages dev.mobile.maestro")?;
        Ok(pkgs.lines().any(|l| l.trim() == "package:dev.mobile.maestro") && pkgs.lines().any(|l| l.trim() == "package:dev.mobile.maestro.test"))
    }

    /// Installs the two driver APKs that ship inside the local Maestro (`maestro-client.jar`) — what a first Maestro
    /// flow does. A wiped emulator, a cold boot image or a new device has none, and no run could read its screen.
    fn install(&self) -> Result<(), String> {
        let jar = maestro_jar().ok_or("Maestro's driver is not installed on the device and no local Maestro was found to install it from (brew install maestro)")?;
        let dir = std::env::temp_dir().join(format!("qapilot-driver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let done = (|| {
            let out = std::process::Command::new("unzip").args(["-o", "-q", "-j"]).arg(&jar).args(DRIVER_APKS).arg("-d").arg(&dir).output().map_err(|e| format!("unzip: {e}"))?;
            if !out.status.success() {
                return Err(format!("{} holds no driver APKs: {}", jar.display(), String::from_utf8_lossy(&out.stderr).trim()));
            }
            for apk in DRIVER_APKS {
                let out = std::process::Command::new(crate::adb::bin()).args(["-s", &self.adb.serial, "install", "-r", "-g", "-t"]).arg(dir.join(apk)).output().map_err(|e| format!("adb: {e}"))?;
                let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                if !out.status.success() || !text.contains("Success") {
                    return Err(format!("installing {apk} on {} failed: {}", self.adb.serial, text.trim()));
                }
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&dir);
        done?;
        if self.installed()? {
            Ok(())
        } else {
            Err("Maestro's driver was installed but the device does not list it".into())
        }
    }

    /// Stops the driver so another automation client (e.g. a Maestro replay) can take the device.
    pub fn release(&self) -> Result<(), String> {
        self.adb.shell("am force-stop dev.mobile.maestro.test; am force-stop dev.mobile.maestro").map(|_| ())
    }

    pub fn device_info(&self) -> Result<(u32, u32), String> {
        let r = self.call("deviceInfo", Bytes::new(), Duration::from_secs(5))?;
        let f = pb::fields(r);
        let get = |n| f.iter().find(|(k, _)| *k == n).and_then(|(_, v)| v.as_ref().ok().copied()).unwrap_or(0) as u32;
        Ok((get(1), get(2)))
    }

    /// The UI hierarchy as XML (uiautomator format), read without waiting for the app to be idle.
    pub fn hierarchy(&self) -> Result<String, String> {
        let r = self.call("viewHierarchy", Bytes::new(), Duration::from_secs(15))?;
        for (k, v) in pb::fields(r) {
            if k == 1 {
                if let Err(b) = v {
                    return Ok(String::from_utf8_lossy(&b).into_owned());
                }
            }
        }
        Ok(String::new())
    }

    pub fn input_text(&self, text: &str) -> Result<(), String> {
        let mut b = BytesMut::new();
        pb::string(&mut b, 1, text.as_bytes());
        self.call("inputText", b.freeze(), Duration::from_secs(20)).map(|_| ())
    }

    pub fn erase(&self, n: u32) -> Result<(), String> {
        let mut b = BytesMut::new();
        pb::uint(&mut b, 1, n as u64);
        self.call("eraseAllText", b.freeze(), Duration::from_secs(10)).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::pb;
    use bytes::BytesMut;

    #[test]
    fn protobuf_roundtrip() {
        let mut b = BytesMut::new();
        pb::uint(&mut b, 1, 300);
        pb::string(&mut b, 2, b"hello");
        pb::uint(&mut b, 3, 1);
        let f = pb::fields(b.freeze());
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].0, 1);
        assert_eq!(f[0].1.as_ref().ok(), Some(&300));
        assert_eq!(f[1].1.as_ref().err().map(|b| b.as_ref().to_vec()), Some(b"hello".to_vec()));
    }
}
