//! qapilot — a fast driver for Android (emulator / phone) and iOS (iPhone / simulator), for testing mobile apps by hand,
//! from scripts or from an AI agent (MCP).
//! It talks to the adb server's socket and to Maestro's on-device driver over gRPC directly (no JVM per call):
//! a screen read is ~0.2 s and works on animated screens.
//!
//!   qapilot screen                         every visible element (e12 * id | text | desc [bounds]) + fingerprint
//!   qapilot do "<step>" ["<step>" …]       run steps in one go, stop at the first failure, print the final screen
//!   qapilot do --file <flow.steps>         the same from a file, one step per line (# comments) — replays a saved flow
//!   qapilot audit [--out <audit.json>]     tree checks of the current screen: placeholders, labels, tap targets,
//!                                          clipped / truncated text, wrong language, overlapping targets
//!   qapilot shot <file.png|file.jpg>       screenshot (.jpg = 540 px wide)
//!   qapilot snap list|save <name>|load <name>   emulator snapshots
//!   qapilot up | release                   start / stop the on-device driver (release before `maestro test`)
//!   qapilot mcp                            MCP server on stdio (tools: screen, do, screenshot, audit, snapshot, release, design)
//!   qapilot bench                          timings of the basic operations
//!   qapilot design --figma <frame.png> [--figma-meta <get_metadata.xml>] [--figma-scale 1] [--figma-name <node>]
//!                  [--app <screen.png> [--tree <screen.json>] [--app-scale 3]] [--app-name "MyApp 2.3.0"] [--out <dir>] [--tol 2]
//!                                          the screen (live, or a saved one) against its Figma frame: gaps, heights,
//!                                          margins and colours in dp → design.json, design.md, design.png, one picture per finding
//!   qapilot cross --android <check dir> --ios <check dir> [--out <dir>]
//!                                          the same Figma frame on Android and iOS: DESIGN | ANDROID | iOS pictures and cross.md
//! Device: --device <adb serial or iOS UDID>, else $DEVICE, else emulator-5554. An iOS UDID drives the device through the
//! qapilot agent (ios-agent/, built and started on first use).
//! Data (learned screen paths, OTP history, the iOS agent's builds): $QAPILOT_HOME, else ~/.qapilot.
mod adb;
mod audit;
mod cross;
mod design;
mod device;
mod driver;
mod ios;
mod mcp;
mod nav;
mod otp;
mod pics;
mod screen;
mod session;

use session::Session;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

/// Where qapilot keeps its data: $QAPILOT_HOME, else ~/.qapilot.
pub fn home() -> PathBuf {
    std::env::var("QAPILOT_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".qapilot"))
}

/// After a snapshot restore the clock is the snapshot's (hours behind) and servers that check request times refuse the
/// app's requests. The image has no root (`date` cannot be set), so the host's time goes in as a telephony time
/// suggestion — it outranks the stale one that came back with the snapshot. Returns the remaining skew in seconds.
fn sync_clock(adb: &adb::Adb) -> Result<i64, String> {
    let up = adb.shell("cat /proc/uptime")?;
    let host_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis() as i64;
    let up_ms = (up.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()).ok_or("could not read /proc/uptime")? * 1000.0) as i64;
    adb.shell(&format!("cmd time_detector suggest_telephony_time --slot_index 0 --elapsed_realtime {up_ms} --unix_epoch_time {host_ms}"))?;
    let dev = adb.shell("date +%s")?.trim().parse::<i64>().map_err(|e| format!("device date: {e}"))?;
    Ok(dev - host_ms / 1000)
}

/// `adb emu avd snapshot …` (the emulator console; rare, so the adb binary is fine here).
pub fn snapshot(serial: &str, action: &str, name: &str) -> Result<String, String> {
    if ios::is_udid(serial) {
        return Err("snapshots are an Android-emulator feature (an iPhone keeps its login in the app)".into());
    }
    if action != "list" && (name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))) {
        return Err(format!("snapshot name: letters, digits, . _ - only (got \"{name}\")"));
    }
    let mut args = vec!["-s", serial, "emu", "avd", "snapshot", action];
    if action != "list" {
        args.push(name);
    }
    let t0 = Instant::now();
    let out = Command::new(adb::bin()).args(&args).output().map_err(|e| format!("adb: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string();
    if !out.status.success() || text.contains("KO") {
        return Err(format!("snapshot {action} {name}: {text}"));
    }
    if action == "load" {
        // After a restore adb briefly loses the device, and the driver (and its port forward) must be started again.
        let d = driver::Driver::new(serial);
        let ready = (0..150).any(|_| {
            let up = d.adb.shell("getprop sys.boot_completed").map(|v| v.trim() == "1").unwrap_or(false);
            if !up {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            up
        });
        if !ready {
            return Err(format!("snapshot {name} restored, but the device did not come back within 30 s"));
        }
        d.start()?;
        let skew = sync_clock(&d.adb)?;
        if skew.abs() > 5 {
            return Err(format!("snapshot {name} restored, but the device clock is still {skew} s off the host's"));
        }
    }
    Ok(format!("snapshot {action} {name} OK in {:.1}s{}", t0.elapsed().as_secs_f32(), if text.is_empty() || text == "OK" { String::new() } else { format!("\n{text}") }))
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut serial = std::env::var("DEVICE").unwrap_or_else(|_| "emulator-5554".into());
    if let Some(i) = args.iter().position(|a| a == "--device") {
        if i + 1 < args.len() {
            serial = args.remove(i + 1);
            args.remove(i);
        }
    }
    let cmd = args.first().cloned().unwrap_or_default();
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let fail = |e: String| -> ! {
        eprintln!("qapilot: {e}");
        std::process::exit(1)
    };
    match cmd.as_str() {
        "mcp" => mcp::serve(&serial),
        "screen" => {
            let mut s = Session::new(&serial);
            let sc = s.read().unwrap_or_else(|e| fail(e));
            print!("{}", s.render(&sc));
        }
        "do" => {
            let mut s = Session::new(&serial);
            let steps = match rest.first().map(String::as_str) {
                Some("--file") => {
                    let file = rest.get(1).cloned().unwrap_or_else(|| fail("do --file <flow.steps>".into()));
                    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| fail(format!("{file}: {e}")));
                    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect()
                }
                _ => rest.clone(),
            };
            let (log, ok, screen) = s.batch(&steps);
            print!("{log}\n{screen}");
            if !ok {
                std::process::exit(1);
            }
        }
        "audit" => {
            let mut s = Session::new(&serial);
            let out = rest.iter().position(|a| a == "--out").and_then(|i| rest.get(i + 1)).cloned();
            print!("{}", s.audit(out.as_deref()).unwrap_or_else(|e| fail(e)));
        }
        "shot" => {
            let mut s = Session::new(&serial);
            let p = s.shot(&rest.join(" ")).unwrap_or_else(|e| fail(e));
            println!("{p}");
        }
        "snap" => {
            let action = rest.first().map(String::as_str).unwrap_or("list");
            println!("{}", snapshot(&serial, action, rest.get(1).map(String::as_str).unwrap_or("")).unwrap_or_else(|e| fail(e)));
        }
        "up" => {
            let t0 = Instant::now();
            device::Device::new(&serial).start().unwrap_or_else(|e| fail(e));
            println!("driver up ({:.1}s)", t0.elapsed().as_secs_f32());
        }
        "release" => {
            device::Device::new(&serial).release().unwrap_or_else(|e| fail(e));
            println!("driver stopped");
        }
        "bench" => {
            let mut s = Session::new(&serial);
            let t = Instant::now();
            s.d.start().unwrap_or_else(|e| fail(e));
            println!("driver ready        {:>6.0} ms", t.elapsed().as_secs_f64() * 1e3);
            for i in 1..=3 {
                let t = Instant::now();
                let sc = s.read().unwrap_or_else(|e| fail(e));
                println!("screen read #{i}      {:>6.0} ms  ({} elements, fingerprint {})", t.elapsed().as_secs_f64() * 1e3, sc.elements.len(), sc.fingerprint);
            }
            let t = Instant::now();
            let (w, _) = s.d.size().unwrap_or_else(|e| fail(e));
            s.d.tap(w / 2, 12).unwrap_or_else(|e| fail(e));
            println!("tap                 {:>6.0} ms", t.elapsed().as_secs_f64() * 1e3);
            let t = Instant::now();
            let _ = s.settle();
            println!("settle              {:>6.0} ms", t.elapsed().as_secs_f64() * 1e3);
            let t = Instant::now();
            let png = s.d.screenshot_png().unwrap_or_else(|e| fail(e));
            println!("screenshot          {:>6.0} ms  ({} KB)", t.elapsed().as_secs_f64() * 1e3, png.len() / 1024);
            let t = Instant::now();
            let jpg = session::small_jpeg(&png, 540).unwrap_or_else(|e| fail(e));
            println!("→ 540 px JPEG       {:>6.0} ms  ({} KB)", t.elapsed().as_secs_f64() * 1e3, jpg.len() / 1024);
        }
        "design" => {
            let arg = |k: &str| rest.iter().position(|a| a == k).and_then(|i| rest.get(i + 1)).cloned();
            let a = design::Args {
                figma: arg("--figma").unwrap_or_else(|| fail("--figma <frame.png> is required".into())),
                figma_meta: arg("--figma-meta"),
                figma_scale: arg("--figma-scale").and_then(|v| v.parse().ok()),
                figma_name: arg("--figma-name"),
                app: arg("--app"),
                tree: arg("--tree"),
                app_scale: arg("--app-scale").and_then(|v| v.parse().ok()),
                out: arg("--out").unwrap_or_else(|| "design".into()),
                tol: arg("--tol").and_then(|v| v.parse().ok()),
                app_name: arg("--app-name"),
            };
            print!("{}", design::check(&serial, &a).unwrap_or_else(|e| fail(e)));
        }
        "cross" => {
            let arg = |k: &str| rest.iter().position(|a| a == k).and_then(|i| rest.get(i + 1)).cloned();
            let (a, i) = (arg("--android").unwrap_or_else(|| fail("--android <dir of the Android check> is required".into())), arg("--ios").unwrap_or_else(|| fail("--ios <dir of the iOS check> is required".into())));
            print!("{}", cross::cross(&a, &i, &arg("--out").unwrap_or_else(|| "cross".into())).unwrap_or_else(|e| fail(e)));
        }
        _ => {
            eprintln!("usage: qapilot [--device <serial>] screen | do \"<step>\"… | do --file <flow.steps> | audit | shot <file> [<sel>] | snap list|save|load <name> | up | release | mcp | bench | design --figma <png> …");
            std::process::exit(2);
        }
    }
}
