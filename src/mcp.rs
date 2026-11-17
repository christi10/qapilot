//! MCP server over stdio (JSON-RPC 2.0, one message per line).
use crate::session::{small_jpeg, Session};
use base64::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const STEPS_HELP: &str = "Steps, one per string: tap <sel> · type <text> · digits <digits> [ms] (PIN/OTP boxes) · erase [n] · back · enter · hidekb · swipe up|down|left|right · scroll <sel> (swipe up until visible) · wait <sel> · gone <sel> · sleep <ms> · launch <package> · stop <package> · otpmark (BEFORE the tap that sends an SMS) · otp <user> [service] (waits for that new OTP from QAPILOT_OTP_CMD and types it) · shot <file.png|file.jpg> [<sel>] (with a selector: cut to that element, the close-up for a bug) · name <screen name> (names the current screen) · goto <screen name> (replays the shortest known path) · issue <KEY> (the steps after it belong to that issue: its replay flow is saved from them). A tap, swipe, launch and goto already wait until the screen stops moving and any spinner is gone — follow them with `wait <sel>` for content that loads later, not with `sleep`. Timeouts in seconds as tap:15 / wait:30. Selectors: e12 (ref from the last screen) · id:<resource-id> · text:<contains> · desc:<contains> · re:<regex on text or desc> · plain words (text or desc contains, case-insensitive).";

fn tools() -> Value {
    json!([
        { "name": "screen", "description": "The current screen: every visible element with text / description / id as `e12 * id | text | desc [bounds]` (* = tappable), the screen fingerprint and its name if known. ~0.2 s, works on animated screens.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "do", "description": format!("Runs steps in order on the device in ONE call and stops at the first failure; returns a log and the final screen. Do a whole intent per call (fill a form, open a screen, enter an OTP) instead of one tap per call. {STEPS_HELP}"), "inputSchema": { "type": "object", "properties": { "steps": { "type": "array", "items": { "type": "string" } } }, "required": ["steps"] } },
        { "name": "screenshot", "description": "A screenshot to look at (540 px wide JPEG). Pass `save` (a .png path) to also keep a full-size copy as evidence.", "inputSchema": { "type": "object", "properties": { "save": { "type": "string" } } } },
        { "name": "audit", "description": "Tree checks of the CURRENT screen, the same answer every run, in milliseconds: values that never reached the screen (`undefined`, `{{name}}`, a translation key), tap targets with no accessibility label, tap targets under 24 dp (certain) or under the platform minimum (to look at), texts cut off at the screen edge or with an ellipsis, texts in the other language than the screen, overlapping tap targets. Findings are tagged [kind | CERTAIN/UNCERTAIN | rule] with the element and its selector. Run it once on every screen you visit. Pass `save` (a .json path) to keep the findings for the report.", "inputSchema": { "type": "object", "properties": { "save": { "type": "string" } } } },
        { "name": "snapshot", "description": "Emulator snapshots: `list`, `save <name>` (e.g. after logging a test user in), `load <name>` (back to that exact state in seconds — no onboarding).", "inputSchema": { "type": "object", "properties": { "action": { "type": "string", "enum": ["list", "save", "load"] }, "name": { "type": "string" } }, "required": ["action"] } },
        { "name": "release", "description": "Stops the on-device driver so a Maestro flow (`maestro test`) can take the device. The next screen/do call starts it again by itself.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "design", "description": "Design check of the CURRENT screen (or a saved screenshot) against its Figma frame, measured deterministically in dp in < 1 s: visual blocks are found on both sides and aligned (sections that exist on one side only, e.g. an empty state, are skipped and noted), then gaps between neighbouring blocks, heights, left margins, content cut off at the screen edge and text colours (CIEDE2000) are compared. With the Figma metadata (get_metadata XML saved to a file) and the live app tree, gaps are measured on layout boxes — the numbers design bugs quote. Returns findings tagged [category | CERTAIN/UNCERTAIN] and writes design.json, design.png (overview) and one FIGMA | APP picture per finding into `out`. CERTAIN findings are measurements you can report; look at the picture of an UNCERTAIN one before reporting it.", "inputSchema": { "type": "object", "properties": {
            "figma": { "type": "string", "description": "the Figma frame PNG (get_screenshot of the frame, saved with figma-png)" },
            "figma_meta": { "type": "string", "description": "the frame's get_metadata XML saved to a file (optional, gives element names and layout boxes)" },
            "figma_name": { "type": "string", "description": "label for the pictures, e.g. the node id 12:345" },
            "figma_scale": { "type": "number", "description": "px per pt of the Figma PNG (default 1 = get_screenshot)" },
            "app": { "type": "string", "description": "a saved app screenshot instead of the live screen (optional)" },
            "tree": { "type": "string", "description": "a saved qapilot screen JSON for `app` (optional)" },
            "out": { "type": "string", "description": "folder for the results, e.g. RUN/design/invest" },
            "tol": { "type": "number", "description": "dp difference that counts (default 2)" },
            "app_name": { "type": "string", "description": "the app and version for the pictures, e.g. \"MyApp 2.3.0 (230)\"" }
        }, "required": ["figma", "out"] } }
    ])
}

fn text(s: String) -> Value {
    json!({ "content": [{ "type": "text", "text": s }] })
}

fn call(s: &mut Session, serial: &str, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "screen" => {
            let sc = s.read()?;
            Ok(text(s.render(&sc)))
        }
        "do" => {
            let steps: Vec<String> = args.get("steps").and_then(|v| v.as_array()).ok_or("steps: [..] missing")?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
            let (log, ok, screen) = s.batch(&steps);
            let mut v = text(format!("{log}\n{screen}"));
            if !ok {
                v["isError"] = json!(true);
            }
            Ok(v)
        }
        "screenshot" => {
            let png = s.d.screenshot_png()?;
            if let Some(p) = args.get("save").and_then(|v| v.as_str()) {
                if let Some(dir) = std::path::Path::new(p).parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                std::fs::write(p, &png).map_err(|e| format!("{p}: {e}"))?;
            }
            let jpg = small_jpeg(&png, 540)?;
            Ok(json!({ "content": [{ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(jpg), "mimeType": "image/jpeg" }] }))
        }
        "audit" => s.audit(args.get("save").and_then(|v| v.as_str())).map(text),
        "snapshot" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            crate::snapshot(serial, action, name).map(text)
        }
        "design" => {
            let st = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::to_string);
            let num = |k: &str| args.get(k).and_then(|v| v.as_f64()).map(|v| v as f32);
            let a = crate::design::Args {
                figma: st("figma").ok_or("figma: path of the Figma frame PNG missing")?,
                figma_meta: st("figma_meta"),
                figma_scale: num("figma_scale"),
                figma_name: st("figma_name"),
                app: st("app"),
                tree: st("tree"),
                app_scale: None,
                out: st("out").ok_or("out: results folder missing")?,
                tol: num("tol"),
                app_name: st("app_name"),
            };
            crate::design::check(serial, &a).map(text)
        }
        "release" => {
            s.d.release()?;
            Ok(text("driver stopped — Maestro can use the device now".into()))
        }
        other => Err(format!("unknown tool {other}")),
    }
}

pub fn serve(serial: &str) {
    let mut s = Session::new(serial);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = req.get("id").cloned() else { continue }; // notifications need no answer
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));
        let reply = match method {
            "initialize" => json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")), "capabilities": { "tools": {} }, "serverInfo": { "name": "qapilot", "version": env!("CARGO_PKG_VERSION") } } }),
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            "tools/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools() } }),
            "tools/call" => {
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let result = call(&mut s, serial, name, &args).unwrap_or_else(|e| json!({ "content": [{ "type": "text", "text": e }], "isError": true }));
                json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
            other => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {other}") } }),
        };
        let _ = writeln!(out, "{reply}");
        let _ = out.flush();
    }
}
