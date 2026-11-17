//! SMS OTPs of test users, from a command you plug in: `QAPILOT_OTP_CMD` prints the newest OTPs as a JSON array
//! `[{ "id": "…", "code": "123456", "user": "…", "phone": "…", "service": "…", "time": "2026-11-17T10:00:00Z" }]`
//! (only `id` and `code` are required). A feed that lists only the last few codes of every tester is fine: a code is
//! "new since the mark, for this user (+ service)".
use serde_json::Value;
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Runs QAPILOT_OTP_CMD (10 s at most) and returns the OTPs it printed.
pub fn fetch() -> Result<Vec<Value>, String> {
    let cmd = std::env::var("QAPILOT_OTP_CMD").unwrap_or_default();
    if cmd.trim().is_empty() {
        return Err("QAPILOT_OTP_CMD is not set — a command that prints the newest OTPs as JSON (see the README)".into());
    }
    let mut child = Command::new("/bin/sh").arg("-c").arg(&cmd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("OTP command: {e}"))?;
    let t0 = Instant::now();
    while child.try_wait().map_err(|e| format!("OTP command: {e}"))?.is_none() {
        if t0.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            return Err("OTP command: no answer within 10 s".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().map_err(|e| format!("OTP command: {e}"))?;
    if !out.status.success() {
        return Err(format!("OTP command failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("OTP command: not a JSON array of OTPs ({e})"))
}

fn field(o: &Value, k: &str) -> String {
    o.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default()
}

/// The ids visible now — call before the tap that sends the SMS.
pub fn mark() -> Result<HashSet<String>, String> {
    Ok(fetch()?.iter().map(|o| field(o, "id")).collect())
}

pub fn matches(o: &Value, user: &str, service: &str) -> bool {
    let user_ok = user.is_empty() || field(o, "user").eq_ignore_ascii_case(user) || field(o, "phone").contains(user);
    let svc_ok = service.is_empty() || field(o, "service").to_lowercase().contains(&service.to_lowercase());
    user_ok && svc_ok
}

/// Waits for a code that was not in `seen`, for this user (+ service): polls every ~0.4 s up to `timeout`.
pub fn wait(seen: &HashSet<String>, user: &str, service: &str, timeout: Duration) -> Result<String, String> {
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        let mut hits: Vec<Value> = fetch()?.into_iter().filter(|o| !seen.contains(&field(o, "id")) && matches(o, user, service)).collect();
        hits.sort_by_key(|o| std::cmp::Reverse(field(o, "time")));
        if let Some(code) = hits.first().map(|o| field(o, "code")).filter(|c| !c.is_empty()) {
            return Ok(code);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err(format!("no new OTP for {user}{} within {} s", if service.is_empty() { String::new() } else { format!(" ({service})") }, timeout.as_secs()))
}

#[cfg(test)]
mod tests {
    use super::matches;
    use serde_json::json;

    #[test]
    fn picks_by_user_and_service() {
        let o = json!({"id": 1, "code": "123456", "user": "user1", "phone": "+15550100123", "service": "/login/activate"});
        assert!(matches(&o, "user1", "login"));
        assert!(matches(&o, "0123", ""));
        assert!(!matches(&o, "user2", ""));
        assert!(!matches(&o, "user1", "device/register"));
    }
}
