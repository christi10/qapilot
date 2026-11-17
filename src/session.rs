//! Steps on the device and batches of them. A step is one line:
//!   tap <sel> · type <text> · digits <digits> [ms] · erase [n] · back · enter · home · hidekb
//!   swipe up|down|left|right · scroll <sel> · wait <sel> · gone <sel> · sleep <ms>
//!   launch <package> · stop <package> · otpmark · otp <user> [service] · shot <file.png|.jpg> [<sel>]
//!   name <screen name> · goto <screen name> · issue <KEY>
//! `tap`, `wait`, `gone`, `scroll`, `otp` and `goto` take a timeout in seconds as `:N` (`wait:30 Home`).
//! Selectors (see screen::find): e12 · id:x · text:… · desc:… · re:<regex> · plain words.
//!
//! With `QAPILOT_JOURNAL=<file>` every step that worked is appended to that file as one JSON line, in a form that
//! replays (`tap e12` as `tap id:…`), under the issue the last `issue <KEY>` step named — the run's replay flows are
//! cut from it, so nobody has to write them.
use crate::device::Device;
use crate::nav::Nav;
use crate::otp;
use crate::screen::{El, Screen};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct Session {
    pub d: Device,
    pub last: Option<Screen>,
    otp_seen: Option<HashSet<String>>,
    nav: Option<Nav>,
    /// the last step as it replays, when that differs from what was asked (`tap e12` → `tap id:login-button`)
    replay: Option<String>,
    /// the step before this one ended with `settle` (a tap, swipe, launch): a `sleep` after it adds nothing
    settled: bool,
}

/// What `settle` and `scroll` compare: where every element is. Texts are left out — a clock or a countdown changes
/// them on a screen that is not moving.
fn layout(s: &Screen) -> Vec<[i32; 4]> {
    s.elements.iter().map(|e| e.bounds).collect()
}

/// One line of the journal (`QAPILOT_JOURNAL`); a failure to write it never fails a step.
fn journal(entry: serde_json::Value) {
    let Ok(file) = std::env::var("QAPILOT_JOURNAL") else { return };
    if file.is_empty() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        let _ = writeln!(f, "{entry}");
    }
}

/// Splits `shot` arguments: the file (ends in .png / .jpg / .jpeg) and an optional selector to crop to.
fn shot_args(arg: &str) -> (&str, Option<&str>) {
    for ext in [".png ", ".jpg ", ".jpeg "] {
        if let Some(i) = arg.find(ext) {
            let (file, sel) = arg.split_at(i + ext.len() - 1);
            return (file, Some(sel.trim()).filter(|s| !s.is_empty()));
        }
    }
    (arg, None)
}

fn state_file() -> PathBuf {
    crate::home().join("qapilot-last-screen.json")
}

impl Session {
    pub fn new(serial: &str) -> Session {
        // The previous call's screen, so `tap e12` works across CLI calls without a new read.
        let last = std::fs::read_to_string(state_file()).ok().and_then(|s| serde_json::from_str(&s).ok());
        Session { d: Device::new(serial), last, otp_seen: None, nav: None, replay: None, settled: false }
    }

    fn nav(&mut self, package: &str) -> &mut Nav {
        if self.nav.is_none() {
            self.nav = Some(Nav::load(package));
        }
        self.nav.as_mut().unwrap()
    }

    pub fn screen_name(&mut self, s: &Screen) -> Option<String> {
        let pkg = s.package.clone();
        self.nav(&pkg).name_of(&s.fingerprint).map(str::to_string)
    }

    /// A fresh read of the screen (also kept for `e…` refs).
    pub fn read(&mut self) -> Result<Screen, String> {
        let last_app = self.last.as_ref().map(|s| s.package.clone());
        let s = self.d.screen(last_app.as_deref())?;
        let _ = std::fs::create_dir_all(state_file().parent().unwrap());
        let _ = std::fs::write(state_file(), serde_json::to_string(&s).unwrap_or_default());
        self.last = Some(s.clone());
        Ok(s)
    }

    pub fn render(&mut self, s: &Screen) -> String {
        let name = self.screen_name(s);
        s.render(name.as_deref())
    }

    /// Until the app has finished moving: two reads in a row show the same screen with every element in the same
    /// place (a screen that slides in has its final texts long before its final positions — a tap or a design check
    /// on it would hit the wrong spot), and no spinner. Moving on its own (a carousel) costs 0.6 s, not the full
    /// 2.5 s; a spinner is waited out for up to 10 s.
    pub fn settle(&mut self) -> Result<Screen, String> {
        let t0 = Instant::now();
        let mut prev = self.read()?;
        let mut same_since: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_millis(120));
            let cur = self.read()?;
            let waited = t0.elapsed();
            if cur.busy {
                if waited > Duration::from_secs(10) {
                    return Ok(cur);
                }
            } else if cur.fingerprint == prev.fingerprint {
                let still = layout(&cur) == layout(&prev);
                if still || same_since.is_some_and(|t| t.elapsed() > Duration::from_millis(600)) || waited > Duration::from_millis(2500) {
                    return Ok(cur);
                }
                same_since.get_or_insert_with(Instant::now);
            } else {
                same_since = None;
                if waited > Duration::from_millis(2500) {
                    return Ok(cur);
                }
            }
            prev = cur;
        }
    }

    /// The element for a selector: a ref from the last screen as is, anything else polled until it shows up.
    fn resolve(&mut self, sel: &str, timeout: Duration) -> Result<El, String> {
        if sel.starts_with('e') && sel[1..].chars().all(|c| c.is_ascii_digit()) && sel.len() > 1 {
            return self.last.as_ref().and_then(|s| s.find(sel).ok()?.first().map(|e| (*e).clone())).ok_or(format!("{sel} is not on the last screen — read the screen again"));
        }
        let t0 = Instant::now();
        loop {
            let s = self.read()?;
            if let Some(e) = s.find(sel)?.first() {
                return Ok((*e).clone());
            }
            if t0.elapsed() > timeout {
                return Err(format!("\"{sel}\" not found within {} s", timeout.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    fn keyboard_shown(&self) -> bool {
        self.d.keyboard_shown(self.last.as_ref())
    }

    /// The app on the last screen (iOS types into it).
    fn app(&self) -> String {
        self.last.as_ref().map(|s| s.package.clone()).unwrap_or_default()
    }

    /// Runs one step; returns a short note for the log.
    pub fn step(&mut self, line: &str) -> Result<String, String> {
        let line = line.trim();
        let (head, arg) = line.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((line, ""));
        let (cmd, secs) = match head.split_once(':') {
            Some((c, n)) => (c, n.parse::<u64>().map_err(|_| format!("bad timeout in {head}"))?),
            None => (head, 0),
        };
        let to = |default: u64| Duration::from_secs(if secs > 0 { secs } else { default });
        // `type $TEST_PASSWORD` / `digits $TEST_PIN`: the value comes from the environment and never shows up in a log.
        let secret;
        let arg = match arg.strip_prefix('$').filter(|n| (cmd == "type" || cmd == "digits") && !n.is_empty() && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')) {
            Some(name) => {
                secret = std::env::var(name).map_err(|_| format!("${name} is not set in the environment"))?;
                secret.as_str()
            }
            None => arg,
        };
        match cmd {
            // `tap 769,518` (device pixels) or `tap 66%,20%` (of the screen): for things outside the element tree,
            // e.g. a tour's buttons drawn over the app.
            "tap" if point(arg).is_some() => {
                let (w, h) = match &self.last {
                    Some(s) if s.width > 0 => (s.width, s.height),
                    _ => self.d.size()?,
                };
                let ((x, xp), (y, yp)) = point(arg).unwrap();
                let (x, y) = (if xp { (x * w as f32 / 100.0) as i32 } else { x as i32 }, if yp { (y * h as f32 / 100.0) as i32 } else { y as i32 });
                self.d.tap(x, y)?;
                let after = self.settle()?;
                Ok(format!("tapped ({x},{y}) → screen {}", after.fingerprint))
            }
            "tap" => {
                let before = self.last.as_ref().map(|s| (s.fingerprint.clone(), s.package.clone()));
                let el = self.resolve(arg, to(8))?;
                let before = if arg.starts_with('e') { before } else { self.last.as_ref().map(|s| (s.fingerprint.clone(), s.package.clone())) };
                let (x, y) = el.center();
                self.d.tap(x.max(0), y.max(0))?;
                let after = self.settle()?;
                // learn the move: a stable selector for the element (not a ref, which is only valid once)
                if let (Some((from, pkg)), Some(sel)) = (before, if arg.starts_with('e') { el.selector() } else { Some(arg.to_string()) }) {
                    if pkg == after.package && self.nav(&pkg).record(&from, &after.fingerprint, &format!("tap {sel}")) {
                        self.nav(&pkg).save();
                    }
                }
                if arg.starts_with('e') {
                    // a ref is only valid on the screen it came from: the journal keeps the selector, else the point
                    self.replay = Some(el.selector().map(|sel| format!("tap {sel}")).unwrap_or(format!("tap {x},{y}")));
                }
                Ok(format!("tapped {} ({x},{y}) → screen {}", el.selector().unwrap_or(el.r#ref.clone()), after.fingerprint))
            }
            "type" => {
                self.d.input_text(arg, &self.app())?;
                Ok(format!("typed {} characters", arg.chars().count()))
            }
            "digits" => {
                let (digits, ms) = arg.split_once(' ').map(|(a, b)| (a, b.trim().parse().unwrap_or(350))).unwrap_or((arg, 350));
                // An on-screen keypad (custom PIN pads take no keyboard input): tap its keys; otherwise type into the focused box.
                let sc = self.read()?;
                let key = |c: char| sc.elements.iter().chain(&sc.keyboard).filter(|e| e.text == c.to_string() || e.desc == c.to_string()).max_by_key(|e| e.bounds[1]).map(|e| e.center());
                let keys: Option<Vec<(i32, i32)>> = digits.chars().map(key).collect();
                if let Some(keys) = keys {
                    for (x, y) in keys {
                        self.d.tap(x, y)?;
                        std::thread::sleep(Duration::from_millis(ms));
                    }
                    return Ok(format!("tapped {} digits on the on-screen keypad", digits.chars().count()));
                }
                let app = self.app();
                for c in digits.chars() {
                    self.d.input_text(&c.to_string(), &app)?;
                    std::thread::sleep(Duration::from_millis(ms));
                }
                Ok(format!("typed {} digits one by one", digits.chars().count()))
            }
            "erase" => {
                self.d.erase(arg.parse().unwrap_or(50), &self.app())?;
                Ok("erased".into())
            }
            "back" | "enter" | "home" => self.d.key(cmd, &self.app()).map(|_| cmd.to_string()),
            "hidekb" => {
                if self.keyboard_shown() {
                    // Android: back closes the keyboard; iOS: the return key
                    self.d.key(if self.d.is_ios() { "enter" } else { "back" }, &self.app())?;
                    Ok("keyboard hidden".into())
                } else {
                    Ok("no keyboard".into())
                }
            }
            // `swipe 50%,99%,50%,40%` (pixels or % of the screen): a gesture from one exact point to another,
            // e.g. the iOS unlock swipe that has to start at the bottom edge.
            "swipe" if arg.matches(',').count() == 3 => {
                let (w, h) = match &self.last {
                    Some(s) if s.width > 0 => (s.width, s.height),
                    _ => self.d.size()?,
                };
                let p: Vec<&str> = arg.split(',').collect();
                let a = point(&format!("{},{}", p[0], p[1])).ok_or(format!("swipe x1,y1,x2,y2? got {arg}"))?;
                let b = point(&format!("{},{}", p[2], p[3])).ok_or(format!("swipe x1,y1,x2,y2? got {arg}"))?;
                let px = |(v, pc): (f32, bool), full: i32| if pc { (v * full as f32 / 100.0) as i32 } else { v as i32 };
                let (x1, y1, x2, y2) = (px(a.0, w), px(a.1, h), px(b.0, w), px(b.1, h));
                self.d.swipe(x1, y1, x2, y2, 300)?;
                self.settle()?;
                Ok(format!("swiped ({x1},{y1}) → ({x2},{y2})"))
            }
            "swipe" | "scroll" => {
                let (w, h) = match &self.last {
                    Some(s) if s.width > 0 => (s.width, s.height),
                    _ => self.d.size()?,
                };
                let line = |dir: &str| -> Result<(i32, i32, i32, i32), String> {
                    let (cx, cy) = (w / 2, h / 2);
                    Ok(match dir {
                        "up" => (cx, h * 7 / 10, cx, h * 3 / 10),
                        "down" => (cx, h * 3 / 10, cx, h * 7 / 10),
                        "left" => (w * 8 / 10, cy, w * 2 / 10, cy),
                        "right" => (w * 2 / 10, cy, w * 8 / 10, cy),
                        other => return Err(format!("swipe {other}? (up|down|left|right)")),
                    })
                };
                if cmd == "swipe" {
                    let (x1, y1, x2, y2) = line(if arg.is_empty() { "up" } else { arg })?;
                    self.d.swipe(x1, y1, x2, y2, 300)?;
                    self.settle()?;
                    return Ok(format!("swiped {}", if arg.is_empty() { "up" } else { arg }));
                }
                // Down the page first, then back up past where it started: the element may be above. A swipe that
                // moves nothing is the end of the page — no point in the other eleven.
                let mut swipes = 0;
                for dir in ["up", "down"] {
                    let mut before = self.read()?;
                    for _ in 0..(if dir == "up" { 12 } else { 24 }) {
                        if !before.find(arg)?.is_empty() {
                            return Ok(format!("\"{arg}\" visible after {swipes} swipes"));
                        }
                        let (x1, y1, x2, y2) = line(dir)?;
                        self.d.swipe(x1, y1, x2, y2, 300)?;
                        swipes += 1;
                        let after = self.settle()?;
                        let end = after.fingerprint == before.fingerprint && layout(&after) == layout(&before);
                        before = after;
                        if end {
                            break;
                        }
                    }
                    if !before.find(arg)?.is_empty() {
                        return Ok(format!("\"{arg}\" visible after {swipes} swipes"));
                    }
                }
                Err(format!("\"{arg}\" is not on this page (scrolled to both ends, {swipes} swipes)"))
            }
            "wait" => {
                let el = self.resolve(arg, to(15))?;
                Ok(format!("\"{arg}\" is there ({})", el.r#ref))
            }
            "gone" => {
                let t0 = Instant::now();
                loop {
                    if self.read()?.find(arg)?.is_empty() {
                        return Ok(format!("\"{arg}\" is gone"));
                    }
                    if t0.elapsed() > to(15) {
                        return Err(format!("\"{arg}\" still there after {} s", to(15).as_secs()));
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
            }
            "sleep" => {
                std::thread::sleep(Duration::from_millis(arg.parse().unwrap_or(500)));
                Ok(if self.settled { "slept — the step before already waited for the screen to stop moving; for content that loads later `wait <selector>` returns as soon as it is there".into() } else { "slept".into() })
            }
            "launch" => {
                self.d.launch(shell_safe(arg)?)?;
                self.settle()?;
                Ok(format!("launched {arg}"))
            }
            "stop" => self.d.stop(shell_safe(arg)?).map(|_| format!("stopped {arg}")),
            "otpmark" => {
                let seen = otp::mark()?;
                // Kept on disk too, so `otp` also works from a later CLI call.
                let _ = std::fs::write(otp_mark_file(), serde_json::to_string(&seen).unwrap_or_default());
                self.otp_seen = Some(seen);
                Ok("OTPs marked".into())
            }
            "otp" => {
                let mut it = arg.split_whitespace();
                let user = it.next().ok_or("otp <user> [service]")?;
                let service = it.next().unwrap_or("");
                let seen = match self.otp_seen.take() {
                    Some(s) => s,
                    None => std::fs::read_to_string(otp_mark_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).ok_or("no `otpmark` before the step that sends the SMS")?,
                };
                let _ = std::fs::remove_file(otp_mark_file());
                let code = otp::wait(&seen, user, service, to(120))?;
                // Like `digits`: an on-screen keypad is tapped, else the focused field gets the code.
                self.step(&format!("digits {code}"))?;
                Ok(format!("OTP for {user}{} typed", if service.is_empty() { String::new() } else { format!(" ({service})") }))
            }
            "shot" => self.shot(arg).map(|p| format!("screenshot {p}")),
            // `issue PRJ-12`: what follows is that issue's test (the journal groups the steps by it).
            "issue" => {
                if arg.is_empty() || arg.len() > 40 || !arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err(format!("issue <KEY>? got \"{arg}\""));
                }
                Ok(format!("steps from here belong to {arg}"))
            }
            "name" => {
                let s = self.read()?;
                let pkg = s.package.clone();
                self.nav(&pkg).set_name(&s.fingerprint, arg);
                self.nav(&pkg).save();
                Ok(format!("screen {} named \"{arg}\"", s.fingerprint))
            }
            "goto" => {
                let s = self.read()?;
                let pkg = s.package.clone();
                let targets = self.nav(&pkg).targets(arg);
                if targets.is_empty() {
                    return Err(format!("no screen named \"{arg}\" yet — reach it once and `name` it"));
                }
                let path = self.nav(&pkg).path(&s.fingerprint, &targets).ok_or(format!("no known way from screen {} to \"{arg}\"", s.fingerprint))?;
                for (i, e) in path.iter().enumerate() {
                    self.step(&format!("{} {}", e.step.split_whitespace().next().unwrap_or("tap"), e.step.split_once(' ').map(|x| x.1).unwrap_or("")))
                        .map_err(|err| format!("goto stopped at hop {} ({}): {err}", i + 1, e.step))?;
                    let now = self.last.as_ref().map(|s| s.fingerprint.clone()).unwrap_or_default();
                    if now != e.to {
                        return Err(format!("goto stopped at hop {}: expected screen {} after `{}`, got {now}", i + 1, e.to, e.step));
                    }
                }
                Ok(format!("at \"{arg}\" in {} steps", path.len()))
            }
            "" => Ok(String::new()),
            other => Err(format!("unknown step `{other}`")),
        }
    }

    /// A screenshot (adb screencap / the iOS agent): `.png` full size, `.jpg` 540 px wide for reading. With a selector
    /// after the file (`shot evidence/K/02-amount.png id:amount`) the picture is cut to that element and a margin
    /// around it — the close-up a bug report needs, without an image tool.
    pub fn shot(&mut self, arg: &str) -> Result<String, String> {
        let (path, sel) = shot_args(arg);
        let mut png = self.d.screenshot_png()?;
        if let Some(sel) = sel {
            let el = self.resolve(sel, Duration::from_secs(5))?;
            let img = image::load_from_memory(&png).map_err(|e| format!("screenshot: {e}"))?;
            let (w, h) = (img.width() as i32, img.height() as i32);
            let pad = (w / 16).max(24);
            let (l, t) = ((el.bounds[0] - pad).clamp(0, w - 1), (el.bounds[1] - pad).clamp(0, h - 1));
            let (r, b) = ((el.bounds[2] + pad).clamp(l + 1, w), (el.bounds[3] + pad).clamp(t + 1, h));
            let mut out = std::io::Cursor::new(Vec::new());
            img.crop_imm(l as u32, t as u32, (r - l) as u32, (b - t) as u32).write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("png: {e}"))?;
            png = out.into_inner();
        }
        let p = PathBuf::from(path);
        if let Some(dir) = p.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
        }
        if path.ends_with(".jpg") || path.ends_with(".jpeg") {
            std::fs::write(&p, small_jpeg(&png, 540)?).map_err(|e| e.to_string())?;
        } else {
            std::fs::write(&p, &png).map_err(|e| e.to_string())?;
        }
        Ok(p.display().to_string())
    }

    /// The tree checks of the settled screen (see audit.rs); `out` also keeps them as JSON for the report.
    pub fn audit(&mut self, out: Option<&str>) -> Result<String, String> {
        let sc = self.settle()?;
        let found = crate::audit::audit(&sc, self.d.scale(), self.d.is_ios());
        if let Some(path) = out {
            if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            let name = self.screen_name(&sc);
            let doc = serde_json::json!({ "screen": sc.fingerprint, "name": name, "app": sc.package, "findings": found });
            std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap_or_default()).map_err(|e| format!("{path}: {e}"))?;
        }
        let name = self.screen_name(&sc);
        Ok(crate::audit::render(&sc, name.as_deref(), &found))
    }

    /// Runs steps in order, stops at the first failure; returns (log, ok, final screen rendered).
    pub fn batch(&mut self, steps: &[String]) -> (String, bool, String) {
        let mut log = String::new();
        let mut ok = true;
        for (i, st) in steps.iter().enumerate() {
            let t0 = Instant::now();
            self.replay = None;
            let result = self.step(st);
            let verb = st.trim().split(|c: char| c.is_whitespace() || c == ':').next().unwrap_or("");
            self.settled = result.is_ok() && matches!(verb, "tap" | "swipe" | "launch" | "goto");
            match result {
                Ok(note) => {
                    let ms = t0.elapsed().as_millis() as u64;
                    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                    match st.trim().strip_prefix("issue ") {
                        Some(key) => journal(serde_json::json!({ "at": at, "issue": key.trim() })),
                        None => journal(serde_json::json!({ "at": at, "step": self.replay.take().unwrap_or_else(|| st.trim().to_string()), "ms": ms, "screen": self.last.as_ref().map(|s| s.fingerprint.as_str()).unwrap_or("") })),
                    }
                    log.push_str(&format!("✓ {}. {st} — {note} ({:.1}s)\n", i + 1, ms as f32 / 1000.0));
                }
                Err(e) => {
                    log.push_str(&format!("✗ {}. {st} — {e} ({:.1}s)\n", i + 1, t0.elapsed().as_secs_f32()));
                    if i + 1 < steps.len() {
                        log.push_str(&format!("  ({} later steps not run)\n", steps.len() - i - 1));
                    }
                    ok = false;
                    break;
                }
            }
        }
        let screen = match self.read() {
            Ok(s) => self.render(&s),
            Err(e) => format!("(screen not readable: {e})\n"),
        };
        (log, ok, screen)
    }
}

/// `x,y` with each part a number of pixels or a percentage: ((x, x is %), (y, y is %)).
fn point(arg: &str) -> Option<((f32, bool), (f32, bool))> {
    let (a, b) = arg.split_once(',')?;
    let part = |p: &str| -> Option<(f32, bool)> {
        let p = p.trim();
        match p.strip_suffix('%') {
            Some(n) => n.parse().ok().map(|v| (v, true)),
            None => p.parse().ok().map(|v| (v, false)),
        }
    };
    Some((part(a)?, part(b)?))
}

fn otp_mark_file() -> PathBuf {
    state_file().with_file_name("qapilot-otp-mark.json")
}

/// Package names only (letters, digits, dots, underscores) — they go into a device shell command.
fn shell_safe(s: &str) -> Result<&str, String> {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
        Ok(s)
    } else {
        Err(format!("not a package name: {s}"))
    }
}

pub fn small_jpeg(png: &[u8], width: u32) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(png).map_err(|e| format!("screenshot: {e}"))?;
    let img = if img.width() > width { img.resize(width, u32::MAX, image::imageops::FilterType::Triangle) } else { img };
    let mut out = std::io::Cursor::new(Vec::new());
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 72);
    img.to_rgb8().write_with_encoder(enc).map_err(|e| format!("jpeg: {e}"))?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::shell_safe;
    #[test]
    fn points_in_pixels_or_percent() {
        assert_eq!(super::point("769,518"), Some(((769.0, false), (518.0, false))));
        assert_eq!(super::point("66%, 20.5%"), Some(((66.0, true), (20.5, true))));
        assert_eq!(super::point("text:Login"), None);
        assert_eq!(super::point("Ναι, συνεχίζω"), None);
    }

    #[test]
    fn shot_takes_a_file_and_an_optional_selector() {
        assert_eq!(super::shot_args("evidence/K-1/01-home.png"), ("evidence/K-1/01-home.png", None));
        assert_eq!(super::shot_args("evidence/K-1/02 amount.png id:amount"), ("evidence/K-1/02 amount.png", Some("id:amount")));
        assert_eq!(super::shot_args("a.jpg text:Οι επενδύσεις σου"), ("a.jpg", Some("text:Οι επενδύσεις σου")));
    }

    #[test]
    fn only_package_names_reach_the_shell() {
        assert!(shell_safe("com.example.wallet").is_ok());
        assert!(shell_safe("x; rm -rf /").is_err());
        assert!(shell_safe("").is_err());
    }
}
